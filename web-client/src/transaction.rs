use std::str::FromStr;

#[cfg(feature = "client")]
use serde::ser::SerializeStruct;
use tsify::Tsify;
use wasm_bindgen::prelude::*;
#[cfg(feature = "primitives")]
use wasm_bindgen_derive::TryFromJsValue;

use beserial::{Deserialize, Serialize};
use nimiq_hash::{Blake2bHash, Hash};
#[cfg(feature = "client")]
use nimiq_primitives::policy::Policy;
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};

#[cfg(feature = "client")]
use nimiq_transaction::extended_transaction::ExtendedTransaction;
use nimiq_transaction::{
    account::{
        htlc_contract::CreationTransactionData as HtlcCreationTransactionData,
        staking_contract::IncomingStakingTransactionData,
        vesting_contract::CreationTransactionData as VestingCreationTransactionData,
    },
    TransactionFlags, TransactionFormat,
};
#[cfg(feature = "primitives")]
use nimiq_transaction_builder::TransactionProofBuilder;

use crate::address::Address;
#[cfg(feature = "primitives")]
use crate::key_pair::KeyPair;
use crate::utils::{from_network_id, to_network_id};

/// Transactions describe a transfer of value, usually from the sender to the recipient.
/// However, transactions can also have no value, when they are used to _signal_ a change in the staking contract.
///
/// Transactions can be used to create contracts, such as vesting contracts and HTLCs.
///
/// Transactions require a valid signature proof over their serialized content.
/// Furthermore, transactions are only valid for 2 hours after their validity-start block height.
#[cfg_attr(feature = "primitives", derive(TryFromJsValue))]
#[wasm_bindgen]
#[cfg_attr(feature = "primitives", derive(Clone))]
pub struct Transaction {
    inner: nimiq_transaction::Transaction,
}

#[wasm_bindgen]
impl Transaction {
    /// Creates a new unsigned transaction that transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender to the recipient, where both sender and recipient can be any account type,
    /// and custom extra data can be added to the transaction.
    ///
    /// ### Basic transactions
    /// If both the sender and recipient types are omitted or `0` and both data and flags are empty,
    /// a smaller basic transaction is created.
    ///
    /// ### Extended transactions
    /// If no flags are given, but sender type is not basic (`0`) or data is set, an extended
    /// transaction is created.
    ///
    /// ### Contract creation transactions
    /// To create a new vesting or HTLC contract, set `flags` to `0b1` and specify the contract
    /// type as the `recipient_type`: `1` for vesting, `2` for HTLC. The `data` bytes must have
    /// the correct format of contract creation data for the respective contract type.
    ///
    /// ### Signaling transactions
    /// To interact with the staking contract, signaling transaction are often used to not
    /// transfer any value, but to simply _signal_ a state change instead, such as changing one's
    /// delegation from one validator to another. To create such a transaction, set `flags` to `
    /// 0b10` and populate the `data` bytes accordingly.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when an account type is unknown, the numbers given for value and fee do not fit
    /// within a u64 or the networkId is unknown. Also throws when no data or recipient type is
    /// given for contract creation transactions, or no data is given for signaling transactions.
    #[wasm_bindgen(constructor)]
    pub fn new(
        sender: &Address,
        sender_type: Option<u8>,
        recipient: &Address,
        recipient_type: Option<u8>,
        value: u64,
        fee: u64,
        data: Option<Vec<u8>>,
        flags: Option<u8>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let flags = TransactionFlags::try_from(flags.unwrap_or(0b0))?;

        let tx = if flags.is_empty() {
            // This also creates basic transactions
            nimiq_transaction::Transaction::new_extended(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(0))?,
                Coin::try_from(value)?,
                Coin::try_from(fee)?,
                data.unwrap_or(Vec::new()),
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::CONTRACT_CREATION) {
            nimiq_transaction::Transaction::new_contract_creation(
                data.unwrap_throw(),
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                AccountType::try_from(recipient_type.unwrap_throw())?,
                Coin::try_from(value)?,
                Coin::try_from(fee)?,
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::SIGNALING) {
            nimiq_transaction::Transaction::new_signaling(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(3))?,
                Coin::try_from(fee)?,
                data.unwrap_throw(),
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else {
            return Err(JsError::new("Invalid flags"));
        };

        Ok(Transaction::from_native(tx))
    }

    /// Signs the transaction with the provided key pair. Automatically determines the format
    /// of the signature proof required for the transaction.
    ///
    /// ### Limitations
    /// - HTLC redemption is not supported and will throw.
    /// - Validator deletion transactions are not and cannot be supported.
    /// - For transaction to the staking contract, both signatures are made with the same keypair,
    ///   so it is not possible to interact with a staker that is different from the sender address
    ///   or using a different cold or signing key for validator transactions.
    #[cfg(feature = "primitives")]
    pub fn sign(&self, key_pair: &KeyPair) -> Result<Transaction, JsError> {
        let proof_builder = TransactionProofBuilder::new(self.native_ref().clone());
        let signed_transaction = match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::Vesting(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::Htlc(mut _builder) => {
                // TODO: Create a separate HTLC signing method that takes the type of proof as an argument
                return Err(JsError::new(
                    "HTLC redemption transactions are not supported",
                ));

                // // Redeem
                // let sig = builder.signature_with_key_pair(key_pair);
                // builder.regular_transfer(hash_algorithm, pre_image, hash_count, hash_root, sig);

                // // Refund
                // let sig = builder.signature_with_key_pair(key_pair);
                // builder.timeout_resolve(sig);

                // // Early resolve
                // builder.early_resolve(htlc_sender_signature, htlc_recipient_signature);

                // // Sign early
                // let sig = builder.signature_with_key_pair(key_pair);
                // return Ok(sig);

                // builder.generate().unwrap()
            }
            TransactionProofBuilder::OutStaking(mut builder) => {
                // There is no way to distinguish between an unstaking and validator-deletion transaction
                // from the transaction itself.
                // Validator-deletion transactions are thus not supported.
                // builder.delete_validator(key_pair.native_ref());

                builder.unstake(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::InStaking(mut builder) => {
                // It is possible to add an additional argument `secondary_key_pair: Option<&KeyPair>` with
                // https://docs.rs/wasm-bindgen-derive/latest/wasm_bindgen_derive/#optional-arguments.
                // TODO: Support signing for differing staker and validator signing & cold keys.
                let secondary_key_pair: Option<&KeyPair> = None;

                builder.sign_with_key_pair(secondary_key_pair.unwrap_or(key_pair).native_ref());
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
        };

        Ok(Transaction::from_native(signed_transaction))
    }

    /// Computes the transaction's hash, which is used as its unique identifier on the blockchain.
    pub fn hash(&self) -> String {
        let hash: Blake2bHash = self.inner.hash();
        // TODO: Return an instance of Hash
        hash.to_hex()
    }

    /// Verifies that a transaction has valid properties and a valid signature proof.
    /// Optionally checks if the transaction is valid on the provided network.
    ///
    /// **Throws with any transaction validity error.** Returns without exception if the transaction is valid.
    ///
    /// Throws when the given networkId is unknown.
    pub fn verify(&self, network_id: Option<u8>) -> Result<(), JsError> {
        let network_id = match network_id {
            Some(id) => to_network_id(id)?,
            None => self.inner.network_id,
        };

        self.inner.verify(network_id).map_err(JsError::from)
    }

    /// Tests if the transaction is valid at the specified block height.
    #[wasm_bindgen(js_name = isValidAt)]
    pub fn is_valid_at(&self, block_height: u32) -> bool {
        self.inner.is_valid_at(block_height)
    }

    /// Returns the address of the contract that is created with this transaction.
    #[wasm_bindgen(js_name = getContractCreationAddress)]
    pub fn get_contract_creation_address(&self) -> Address {
        Address::from_native(self.inner.contract_creation_address())
    }

    /// Serializes the transaction's content to be used for creating its signature.
    #[wasm_bindgen(js_name = serializeContent)]
    pub fn serialize_content(&self) -> Vec<u8> {
        self.inner.serialize_content()
    }

    /// Serializes the transaction to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// The transaction's {@link TransactionFormat}.
    #[wasm_bindgen(getter)]
    pub fn format(&self) -> TransactionFormat {
        self.inner.format()
    }

    /// The transaction's sender address.
    #[wasm_bindgen(getter)]
    pub fn sender(&self) -> Address {
        Address::from_native(self.inner.sender.clone())
    }

    /// The transaction's sender {@link AccountType}.
    #[wasm_bindgen(getter, js_name = senderType)]
    pub fn sender_type(&self) -> AccountType {
        self.inner.sender_type
    }

    /// The transaction's recipient address.
    #[wasm_bindgen(getter)]
    pub fn recipient(&self) -> Address {
        Address::from_native(self.inner.recipient.clone())
    }

    /// The transaction's recipient {@link AccountType}.
    #[wasm_bindgen(getter, js_name = recipientType)]
    pub fn recipient_type(&self) -> AccountType {
        self.inner.recipient_type
    }

    /// The transaction's value in luna (NIM's smallest unit).
    #[wasm_bindgen(getter)]
    pub fn value(&self) -> u64 {
        self.inner.value.into()
    }

    /// The transaction's fee in luna (NIM's smallest unit).
    #[wasm_bindgen(getter)]
    pub fn fee(&self) -> u64 {
        self.inner.fee.into()
    }

    /// The transaction's fee per byte in luna (NIM's smallest unit).
    #[wasm_bindgen(getter, js_name = feePerByte)]
    pub fn fee_per_byte(&self) -> f64 {
        self.inner.fee_per_byte()
    }

    /// The transaction's validity-start height. The transaction is valid for 2 hours after this block height.
    #[wasm_bindgen(getter, js_name = validityStartHeight)]
    pub fn validity_start_height(&self) -> u32 {
        self.inner.validity_start_height
    }

    /// The transaction's network ID.
    #[wasm_bindgen(getter, js_name = networkId)]
    pub fn network_id(&self) -> u8 {
        from_network_id(self.inner.network_id)
    }

    /// The transaction's flags: `0b1` = contract creation, `0b10` = signaling.
    #[wasm_bindgen(getter)]
    pub fn flags(&self) -> u8 {
        self.inner.flags.into()
    }

    /// The transaction's data as a byte array.
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.inner.data.clone()
    }

    /// The transaction's signature proof as a byte array.
    #[wasm_bindgen(getter)]
    pub fn proof(&self) -> Vec<u8> {
        self.inner.proof.clone()
    }

    /// Set the transaction's signature proof.
    #[wasm_bindgen(setter)]
    pub fn set_proof(&mut self, proof: Vec<u8>) {
        self.inner.proof = proof;
    }

    /// The transaction's byte size.
    #[wasm_bindgen(getter)]
    pub fn serialized_size(&self) -> usize {
        self.inner.serialized_size()
    }

    /// Serializes the transaction into a HEX string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    /// Creates a JSON-compatible plain object representing the transaction.
    #[wasm_bindgen(js_name = toPlain)]
    pub fn to_plain(&self) -> Result<PlainTransactionType, JsError> {
        let plain = self.to_plain_transaction();
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }

    /// Parses a transaction from a {@link Transaction} instance, a plain object, or a serialized
    /// string representation.
    ///
    /// Throws when a transaction cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(tx: &TransactionAnyType) -> Result<Transaction, JsError> {
        let js_value: &JsValue = tx.unchecked_ref();

        #[cfg(feature = "primitives")]
        if let Ok(transaction) = Transaction::try_from(js_value) {
            return Ok(transaction);
        }

        if let Ok(plain) = serde_wasm_bindgen::from_value::<PlainTransaction>(js_value.clone()) {
            Ok(Transaction::from_plain_transaction(&plain)?)
        } else if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(Transaction::from_native(
                nimiq_transaction::Transaction::deserialize_from_vec(&hex::decode(string)?)?,
            ))
        } else {
            Err(JsError::new("Failed to parse transaction."))
        }
    }

    /// Parses a transaction from a plain object.
    ///
    /// Throws when a transaction cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromPlain)]
    pub fn from_plain(plain: &PlainTransactionType) -> Result<Transaction, JsError> {
        let js_value: &JsValue = plain.unchecked_ref();

        Transaction::from_plain_transaction(&serde_wasm_bindgen::from_value::<PlainTransaction>(
            js_value.to_owned(),
        )?)
    }
}

impl Transaction {
    pub fn from_native(transaction: nimiq_transaction::Transaction) -> Transaction {
        Transaction { inner: transaction }
    }

    pub fn native_ref(&self) -> &nimiq_transaction::Transaction {
        &self.inner
    }

    #[cfg(feature = "client")]
    pub fn native(&self) -> nimiq_transaction::Transaction {
        self.inner.clone()
    }

    #[cfg(feature = "client")]
    pub fn take_native(self) -> nimiq_transaction::Transaction {
        self.inner
    }

    pub fn to_plain_transaction(&self) -> PlainTransaction {
        PlainTransaction {
            transaction_hash: self.hash(),
            format: self.format(),
            sender: self.sender().to_plain(),
            sender_type: self.sender_type(),
            recipient: self.recipient().to_plain(),
            recipient_type: self.recipient_type(),
            value: self.value(),
            fee: self.fee(),
            fee_per_byte: self.fee_per_byte(),
            validity_start_height: self.validity_start_height(),
            network: to_network_id(self.network_id())
                .ok()
                .unwrap()
                .to_string()
                .to_lowercase(),
            flags: self.flags(),
            data: {
                if self.inner.recipient_type == AccountType::Staking {
                    // Parse transaction data
                    let data = IncomingStakingTransactionData::parse(&self.inner).unwrap();
                    match data {
                        IncomingStakingTransactionData::CreateStaker { delegation, .. } => {
                            PlainTransactionData::CreateStaker(PlainCreateStakerData {
                                raw: hex::encode(self.data()),
                                delegation: delegation
                                    .map(|address| address.to_user_friendly_address()),
                            })
                        }
                        IncomingStakingTransactionData::AddStake { staker_address } => {
                            PlainTransactionData::AddStake(PlainAddStakeData {
                                raw: hex::encode(self.data()),
                                staker: staker_address.to_user_friendly_address(),
                            })
                        }
                        IncomingStakingTransactionData::UpdateStaker { new_delegation, .. } => {
                            PlainTransactionData::UpdateStaker(PlainUpdateStakerData {
                                raw: hex::encode(self.data()),
                                new_delegation: new_delegation
                                    .map(|address| address.to_user_friendly_address()),
                            })
                        }
                        IncomingStakingTransactionData::CreateValidator {
                            signing_key,
                            voting_key,
                            reward_address,
                            signal_data,
                            proof_of_knowledge,
                            ..
                        } => PlainTransactionData::CreateValidator(PlainCreateValidatorData {
                            raw: hex::encode(self.data()),
                            signing_key: signing_key.to_hex(),
                            voting_key: voting_key.to_hex(),
                            reward_address: reward_address.to_user_friendly_address(),
                            signal_data: signal_data.map(hex::encode),
                            proof_of_knowledge: proof_of_knowledge.to_hex(),
                        }),
                        IncomingStakingTransactionData::UpdateValidator {
                            new_signing_key,
                            new_voting_key,
                            new_reward_address,
                            new_signal_data,
                            new_proof_of_knowledge,
                            ..
                        } => PlainTransactionData::UpdateValidator(PlainUpdateValidatorData {
                            raw: hex::encode(self.data()),
                            new_signing_key: new_signing_key
                                .map(|signing_key| signing_key.to_hex()),
                            new_voting_key: new_voting_key.map(|voting_key| voting_key.to_hex()),
                            new_reward_address: new_reward_address
                                .map(|reward_address| reward_address.to_user_friendly_address()),
                            new_signal_data: new_signal_data
                                .map(|signal_data| signal_data.map(hex::encode)),
                            new_proof_of_knowledge: new_proof_of_knowledge
                                .map(|proof_of_knowledge| proof_of_knowledge.to_hex()),
                        }),
                        IncomingStakingTransactionData::UnparkValidator {
                            validator_address,
                            ..
                        }
                        | IncomingStakingTransactionData::DeactivateValidator {
                            validator_address,
                            ..
                        }
                        | IncomingStakingTransactionData::ReactivateValidator {
                            validator_address,
                            ..
                        } => PlainTransactionData::GenericValidator(PlainValidatorData {
                            raw: hex::encode(self.data()),
                            validator: validator_address.to_user_friendly_address(),
                        }),
                        IncomingStakingTransactionData::RetireValidator { .. } => {
                            PlainTransactionData::Raw(PlainRawData {
                                raw: hex::encode(self.data()),
                            })
                        }
                    }
                    // In the future we might add other staking notifications
                } else if self.inner.recipient_type == AccountType::Vesting {
                    let data = VestingCreationTransactionData::parse(&self.inner).unwrap();
                    PlainTransactionData::Vesting(PlainVestingData {
                        raw: hex::encode(self.data()),
                        owner: data.owner.to_user_friendly_address(),
                        start_time: data.start_time,
                        step_amount: data.step_amount.into(),
                        time_step: data.time_step,
                    })
                } else if self.inner.recipient_type == AccountType::HTLC {
                    let data = HtlcCreationTransactionData::parse(&self.inner).unwrap();
                    PlainTransactionData::Htlc(PlainHtlcData {
                        raw: hex::encode(self.data()),
                        sender: data.sender.to_user_friendly_address(),
                        recipient: data.recipient.to_user_friendly_address(),
                        hash_algorithm: match data.hash_algorithm {
                            nimiq_transaction::account::htlc_contract::HashAlgorithm::Blake2b => {
                                "blake2b".to_string()
                            }
                            nimiq_transaction::account::htlc_contract::HashAlgorithm::Sha256 => {
                                "sha256".to_string()
                            }
                        },
                        hash_root: hex::encode(data.hash_root),
                        hash_count: data.hash_count,
                        timeout: data.timeout,
                    })
                } else {
                    PlainTransactionData::Raw(PlainRawData {
                        raw: hex::encode(self.data()),
                    })
                }
            },
            proof: PlainTransactionProof {
                raw: hex::encode(self.proof()),
            },
            size: self.serialized_size(),
            valid: self.verify(None).is_ok(),
        }
    }

    pub fn from_plain_transaction(plain: &PlainTransaction) -> Result<Transaction, JsError> {
        let mut tx = Transaction::new(
            &Address::from_string(&plain.sender)?,
            Some(plain.sender_type.into()),
            &Address::from_string(&plain.recipient)?,
            Some(plain.recipient_type.into()),
            plain.value,
            plain.fee,
            Some(hex::decode(match plain.data {
                PlainTransactionData::Raw(ref data) => &data.raw,
                PlainTransactionData::Vesting(ref data) => &data.raw,
                PlainTransactionData::Htlc(ref data) => &data.raw,
                PlainTransactionData::CreateValidator(ref data) => &data.raw,
                PlainTransactionData::UpdateValidator(ref data) => &data.raw,
                PlainTransactionData::GenericValidator(ref data) => &data.raw,
                PlainTransactionData::CreateStaker(ref data) => &data.raw,
                PlainTransactionData::AddStake(ref data) => &data.raw,
                PlainTransactionData::UpdateStaker(ref data) => &data.raw,
            })?),
            Some(plain.flags),
            plain.validity_start_height,
            from_network_id(NetworkId::from_str(&plain.network)?),
        )?;
        tx.set_proof(hex::decode(&plain.proof.raw)?);

        Ok(tx)
    }
}

/// Placeholder struct to serialize data of transactions as hex strings in the style of the Nimiq 1.0 library.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(untagged)]
pub enum PlainTransactionData {
    Raw(PlainRawData),
    Vesting(PlainVestingData),
    Htlc(PlainHtlcData),
    CreateValidator(PlainCreateValidatorData),
    UpdateValidator(PlainUpdateValidatorData),
    GenericValidator(PlainValidatorData),
    CreateStaker(PlainCreateStakerData),
    AddStake(PlainAddStakeData),
    UpdateStaker(PlainUpdateStakerData),
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
pub struct PlainRawData {
    pub raw: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainVestingData {
    pub raw: String,
    pub owner: String,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: u64,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcData {
    pub raw: String,
    pub sender: String,
    pub recipient: String,
    pub hash_algorithm: String,
    pub hash_root: String,
    pub hash_count: u8,
    pub timeout: u64,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainCreateValidatorData {
    raw: String,
    signing_key: String,
    voting_key: String,
    reward_address: String,
    signal_data: Option<String>,
    proof_of_knowledge: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainUpdateValidatorData {
    raw: String,
    new_signing_key: Option<String>,
    new_voting_key: Option<String>,
    new_reward_address: Option<String>,
    new_signal_data: Option<Option<String>>,
    new_proof_of_knowledge: Option<String>,
}

// Used for UnparkValidator, DeactivateValidator & ReactivateValidator, as they have the same fields.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainValidatorData {
    pub raw: String,
    pub validator: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainCreateStakerData {
    pub raw: String,
    pub delegation: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainAddStakeData {
    pub raw: String,
    pub staker: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainUpdateStakerData {
    pub raw: String,
    pub new_delegation: Option<String>,
}

/// Placeholder struct to serialize proofs of transactions as hex strings in the style of the Nimiq 1.0 library.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
pub struct PlainTransactionProof {
    pub raw: String,
}

/// JSON-compatible and human-readable format of transactions. E.g. addresses are presented in their human-readable
/// format and address types and the network are represented as strings. Data and proof are serialized as an object
/// describing their contents (not yet implemented, only the `{ raw: string }` fallback is available).
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransaction {
    /// The transaction's unique hash, used as its identifier. Sometimes also called `txId`.
    pub transaction_hash: String,
    /// The transaction's format. Nimiq transactions can have one of two formats: "basic" and "extended".
    /// Basic transactions are simple value transfers between two regular address types and cannot contain
    /// any extra data. Basic transactions can be serialized to less bytes, so take up less place on the
    /// blockchain. Extended transactions on the other hand are all other transactions: contract creations
    /// and interactions, staking transactions, transactions with exta data, etc.
    #[tsify(type = "PlainTransactionFormat")]
    pub format: TransactionFormat,
    /// The transaction's sender address in human-readable IBAN format.
    pub sender: String,
    /// The type of the transaction's sender. "basic" are regular private-key controlled addresses,
    /// "vesting" and "htlc" are those contract types respectively, and "staking" is the staking contract.
    #[tsify(type = "PlainAccountType")]
    pub sender_type: AccountType,
    /// The transaction's recipient address in human-readable IBAN format.
    pub recipient: String,
    /// The type of the transaction's sender. "basic" are regular private-key controlled addresses,
    /// "vesting" and "htlc" are those contract types respectively, and "staking" is the staking contract.
    #[tsify(type = "PlainAccountType")]
    pub recipient_type: AccountType,
    // The transaction's value in luna (NIM's smallest unit).
    pub value: u64,
    /// The transaction's fee in luna (NIM's smallest unit).
    pub fee: u64,
    /// The transaction's fee-per-byte in luna (NIM's smallest unit).
    pub fee_per_byte: f64,
    /// The block height at which this transaction becomes valid. It is then valid for 7200 blocks (~2 hours).
    pub validity_start_height: u32,
    /// The network name on which this transaction is valid.
    pub network: String,
    /// Any flags that this transaction carries. `0b1 = 1` means it's a contract-creation transaction, `0b10 = 2`
    /// means it's a signalling transaction with 0 value.
    pub flags: u8,
    /// The `data` field of a transaction serves different purposes based on the transaction's recipient type.
    /// For transactions to "basic" address types, this field can contain up to 64 bytes of unstructured data.
    /// For transactions that create contracts or interact with the staking contract, the format of this field
    /// must follow a fixed structure and defines the new contracts' properties or how the staking contract is
    /// changed.
    pub data: PlainTransactionData,
    /// The `proof` field contains the signature of the eligible signer. The proof field's structure depends on
    /// the transaction's sender type. For transactions from contracts it can also contain additional structured
    /// data before the signature.
    pub proof: PlainTransactionProof,
    /// The transaction's serialized size in bytes. It is used to determine the fee-per-byte that this
    /// transaction pays.
    pub size: usize,
    /// Encodes if the transaction is valid, meaning the signature is valid and the `data` and `proof` fields
    /// follow the correct format for the transaction's recipient and sender type, respectively.
    pub valid: bool,
}

/// Describes the state of a transaction as known by the client.
#[cfg(feature = "client")]
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "lowercase")]
pub enum TransactionState {
    /// The transaction only exists locally and has not been broadcast or accepted by any peers.
    New,
    /// The transaction has been broadcast and accepted by peers and is waiting in the mempool for
    /// inclusion into the blockchain.
    Pending,
    /// The transaction has been included into the blockchain, but not yet finalized by a following
    /// macro block.
    Included,
    /// The transaction is included in the blockchain and has been finalized by a following macro block.
    Confirmed,
    /// The transaction was invalided by a blockchain state change before it could be included into
    /// the chain, or was replaced by a higher-fee transaction, or cannot be applied anymore after a
    /// blockchain rebranch.
    Invalidated,
    /// The transaction's validity window has expired and the transaction can no longer be included into
    /// the blockchain.
    Expired,
}

/// JSON-compatible and human-readable format of transactions, including details about its state in the
/// blockchain. Contains all fields from {@link PlainTransaction}, plus additional fields such as
/// `blockHeight` and `timestamp` if the transaction is included in the blockchain.
#[cfg(feature = "client")]
#[derive(Clone, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransactionDetails {
    #[serde(flatten)]
    pub transaction: PlainTransaction,

    pub state: TransactionState,
    #[tsify(optional)]
    pub execution_result: Option<bool>,
    #[tsify(optional)]
    pub block_height: Option<u32>,
    #[tsify(optional)]
    pub confirmations: Option<u32>,
    #[tsify(optional)]
    pub timestamp: Option<u64>,
}

// Manually implement serde::Serialize trait to ensure struct is serialized into a JS Object and not a Map.
//
// Unfortunately, serde cannot serialize a struct that includes a #[serde(flatten)] annotation into an Object,
// and the Github issue for it is closed as "wontfix": https://github.com/serde-rs/serde/issues/1346
#[cfg(feature = "client")]
impl serde::Serialize for PlainTransactionDetails {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut plain = serializer.serialize_struct("PlainTransactionDetails", 21)?;
        plain.serialize_field("transactionHash", &self.transaction.transaction_hash)?;
        plain.serialize_field("format", &self.transaction.format)?;
        plain.serialize_field("sender", &self.transaction.sender)?;
        plain.serialize_field("senderType", &self.transaction.sender_type)?;
        plain.serialize_field("recipient", &self.transaction.recipient)?;
        plain.serialize_field("recipientType", &self.transaction.recipient_type)?;
        plain.serialize_field("value", &self.transaction.value)?;
        plain.serialize_field("fee", &self.transaction.fee)?;
        plain.serialize_field("feePerByte", &self.transaction.fee_per_byte)?;
        plain.serialize_field(
            "validityStartHeight",
            &self.transaction.validity_start_height,
        )?;
        plain.serialize_field("network", &self.transaction.network)?;
        plain.serialize_field("flags", &self.transaction.flags)?;
        plain.serialize_field("data", &self.transaction.data)?;
        plain.serialize_field("proof", &self.transaction.proof)?;
        plain.serialize_field("size", &self.transaction.size)?;
        plain.serialize_field("valid", &self.transaction.valid)?;

        plain.serialize_field("state", &self.state)?;
        plain.serialize_field("executionResult", &self.execution_result)?;
        plain.serialize_field("blockHeight", &self.block_height)?;
        plain.serialize_field("confirmations", &self.confirmations)?;
        plain.serialize_field("timestamp", &self.timestamp)?;
        plain.end()
    }
}

#[cfg(feature = "client")]
impl PlainTransactionDetails {
    // Construct a TransactionDetails struct with all details manually
    pub fn new(
        tx: &Transaction,
        state: TransactionState,
        execution_result: Option<bool>,
        block_height: Option<u32>,
        timestamp: Option<u64>,
        confirmations: Option<u32>,
    ) -> Self {
        Self {
            transaction: tx.to_plain_transaction(),
            state,
            execution_result,
            block_height,
            timestamp,
            confirmations,
        }
    }

    /// Creates a PlainTransactionDetails struct that can be serialized to JS from a native [ExtendedTransaction].
    pub fn from_extended_transaction(ext_tx: &ExtendedTransaction, current_block: u32) -> Self {
        let block_number = ext_tx.block_number;
        let block_time = ext_tx.block_time;

        let last_macro_block = Policy::last_macro_block(current_block);

        let state = if last_macro_block >= block_number {
            TransactionState::Confirmed
        } else {
            TransactionState::Included
        };

        let executed_transaction = ext_tx.clone().into_transaction().unwrap();

        Self {
            transaction: Transaction::from_native(
                executed_transaction.get_raw_transaction().clone(),
            )
            .to_plain_transaction(),
            state,
            execution_result: Some(executed_transaction.succeeded()),
            block_height: Some(block_number),
            timestamp: Some(block_time),
            confirmations: Some(block_number - current_block + 1),
        }
    }
}

/// JSON-compatible and human-readable format of transaction receipts.
#[cfg(feature = "client")]
#[derive(serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransactionReceipt {
    /// The transaction's unique hash, used as its identifier. Sometimes also called `txId`.
    pub transaction_hash: String,
    /// The transaction's block height where it is included in the blockchain.
    pub block_height: u32,
}

#[cfg(feature = "client")]
impl PlainTransactionReceipt {
    /// Creates a PlainTransactionReceipt struct that can be serialized to JS.
    pub fn from_receipt(receipt: &(Blake2bHash, u32)) -> Self {
        Self {
            transaction_hash: receipt.0.to_hex(),
            block_height: receipt.1,
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainTransaction")]
    pub type PlainTransactionType;

    #[wasm_bindgen(typescript_type = "PlainTransactionDetails")]
    pub type PlainTransactionDetailsType;

    #[wasm_bindgen(typescript_type = "PlainTransactionDetails[]")]
    pub type PlainTransactionDetailsArrayType;

    #[wasm_bindgen(typescript_type = "PlainTransactionReceipt")]
    pub type PlainTransactionReceiptType;

    #[wasm_bindgen(typescript_type = "PlainTransactionReceipt[]")]
    pub type PlainTransactionReceiptArrayType;
}

#[cfg(feature = "primitives")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Transaction | PlainTransaction | string")]
    pub type TransactionAnyType;
}

#[cfg(not(feature = "primitives"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainTransaction | string")]
    pub type TransactionAnyType;
}
