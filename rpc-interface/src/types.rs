///! Defines the types used by the JSON RPC API[1]
///!
///! [1] https://github.com/nimiq/core-js/wiki/JSON-RPC-API#common-data-types
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};

use nimiq_block::{MultiSignature, ViewChangeProof};
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::{CompressedPublicKey, CompressedSignature};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_libp2p::PeerId;
use nimiq_primitives::policy;
use nimiq_primitives::slots::Validators;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::account::htlc_contract::AnyHash;
use nimiq_vrf::VrfSeed;

use crate::error::Error;

#[derive(Clone, Debug)]
pub enum BlockNumberOrHash {
    Number(u32),
    Hash(Blake2bHash),
}

impl From<u32> for BlockNumberOrHash {
    fn from(block_number: u32) -> Self {
        BlockNumberOrHash::Number(block_number)
    }
}

impl From<Blake2bHash> for BlockNumberOrHash {
    fn from(block_hash: Blake2bHash) -> Self {
        BlockNumberOrHash::Hash(block_hash)
    }
}

impl Display for BlockNumberOrHash {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            BlockNumberOrHash::Number(block_number) => write!(f, "{}", block_number),
            BlockNumberOrHash::Hash(block_hash) => write!(f, "{}", block_hash),
        }
    }
}

impl FromStr for BlockNumberOrHash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(n) = s.parse::<u32>() {
            Ok(BlockNumberOrHash::Number(n))
        } else {
            Ok(BlockNumberOrHash::Hash(s.parse().map_err(|_| {
                Error::InvalidBlockNumberOrHash(s.to_owned())
            })?))
        }
    }
}

#[derive(Copy, Clone, Debug, SerializeDisplay, DeserializeFromStr)]
pub enum ValidityStartHeight {
    Absolute(u32),
    Relative(u32),
}

impl ValidityStartHeight {
    pub fn block_number(self, current_block_number: u32) -> u32 {
        match self {
            Self::Absolute(n) => n,
            Self::Relative(n) => n + current_block_number,
        }
    }
}

impl Default for ValidityStartHeight {
    fn default() -> Self {
        Self::Relative(0)
    }
}

impl Display for ValidityStartHeight {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Absolute(n) => write!(f, "{}", n),
            Self::Relative(n) => write!(f, "+{}", n),
        }
    }
}

impl FromStr for ValidityStartHeight {
    type Err = <u32 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.starts_with('+') {
            Ok(Self::Relative(s[1..].parse()?))
        } else {
            Ok(Self::Absolute(s.parse()?))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Macro,
    Micro,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub block_type: BlockType,
    pub hash: Blake2bHash,
    pub batch: u32,
    pub epoch: u32,

    pub version: u16,
    pub block_number: u32,
    pub view_number: u32,
    pub timestamp: u64,
    pub parent_hash: Blake2bHash,
    pub seed: VrfSeed,
    #[serde(with = "crate::serde_helpers::hex")]
    pub extra_data: Vec<u8>,
    pub state_root: Blake2bHash,
    pub body_root: Blake2bHash,
    pub history_root: Blake2bHash,

    #[serde(flatten)]
    pub additional_fields: BlockAdditionalFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum BlockAdditionalFields {
    Macro {
        is_election_block: bool,
        proposer: Slot,

        parent_election_hash: Blake2bHash,

        // None if not an election block.
        #[serde(skip_serializing_if = "Option::is_none")]
        slots: Option<Vec<Slots>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lost_reward_set: Option<BitSet>,
        #[serde(skip_serializing_if = "Option::is_none")]
        disabled_set: Option<BitSet>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transactions: Option<Vec<Transaction>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        justification: Option<TendermintProof>,
    },
    Micro {
        producer: Slot,

        #[serde(skip_serializing_if = "Option::is_none")]
        fork_proofs: Option<Vec<ForkProof>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transactions: Option<Vec<Transaction>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        justification: Option<MicroJustification>,
    },
}

impl Block {
    pub fn from_block(
        blockchain: &Blockchain,
        block: nimiq_block::Block,
        include_transactions: bool,
    ) -> Self {
        let block_number = block.block_number();
        let timestamp = block.timestamp();
        let batch = policy::batch_at(block_number);
        let epoch = policy::epoch_at(block_number);

        match block {
            nimiq_block::Block::Macro(macro_block) => {
                let slots = macro_block.get_validators().map(Slots::from_slots);

                let (lost_reward_set, disabled_set) = match macro_block.body.clone() {
                    None => (None, None),
                    Some(body) => (Some(body.lost_reward_set), Some(body.disabled_set)),
                };

                // Get the reward inherents and convert them to reward transactions.
                let ext_txs = blockchain
                    .history_store
                    .get_block_transactions(block_number, None);

                let mut transactions = vec![];

                for ext_tx in ext_txs {
                    if ext_tx.is_inherent() {
                        match ext_tx.into_transaction() {
                            Ok(tx) => {
                                transactions.push(Transaction::from_blockchain(
                                    tx,
                                    block_number,
                                    timestamp,
                                    blockchain.block_number(),
                                ));
                            }
                            Err(_) => {}
                        }
                    }
                }

                Block {
                    block_type: BlockType::Macro,
                    hash: macro_block.hash(),
                    batch,
                    epoch,
                    version: macro_block.header.version,
                    block_number,
                    view_number: macro_block.header.view_number,
                    timestamp,
                    parent_hash: macro_block.header.parent_hash,
                    seed: macro_block.header.seed,
                    extra_data: macro_block.header.extra_data,
                    state_root: macro_block.header.state_root,
                    body_root: macro_block.header.body_root,
                    history_root: macro_block.header.history_root,
                    additional_fields: BlockAdditionalFields::Macro {
                        is_election_block: policy::is_election_block_at(block_number),
                        proposer: Slot::from(
                            blockchain,
                            block_number,
                            macro_block.header.view_number,
                        ),
                        parent_election_hash: macro_block.header.parent_election_hash,
                        slots,
                        lost_reward_set,
                        disabled_set,
                        transactions: Some(transactions),
                        justification: macro_block.justification.map(TendermintProof::from),
                    },
                }
            }

            nimiq_block::Block::Micro(micro_block) => {
                let (fork_proofs, transactions) = match micro_block.body {
                    None => (None, None),
                    Some(ref body) => (
                        Some(
                            body.fork_proofs
                                .clone()
                                .into_iter()
                                .map(Into::into)
                                .collect(),
                        ),
                        if include_transactions {
                            let head_height = blockchain.block_number();
                            Some(
                                body.transactions
                                    .clone()
                                    .into_iter()
                                    .map(|tx| {
                                        Transaction::from_blockchain(
                                            tx,
                                            block_number,
                                            timestamp,
                                            head_height,
                                        )
                                    })
                                    .collect(),
                            )
                        } else {
                            None
                        },
                    ),
                };

                Block {
                    block_type: BlockType::Micro,
                    hash: micro_block.hash(),
                    batch,
                    epoch,
                    version: micro_block.header.version,
                    block_number,
                    view_number: micro_block.header.view_number,
                    timestamp,
                    parent_hash: micro_block.header.parent_hash,
                    seed: micro_block.header.seed,
                    extra_data: micro_block.header.extra_data,
                    state_root: micro_block.header.state_root,
                    body_root: micro_block.header.body_root,
                    history_root: micro_block.header.history_root,
                    additional_fields: BlockAdditionalFields::Micro {
                        producer: Slot::from(
                            blockchain,
                            block_number,
                            micro_block.header.view_number,
                        ),
                        fork_proofs,
                        transactions,
                        justification: micro_block.justification.map(Into::into),
                    },
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TendermintProof {
    round: u32,
    sig: MultiSignature,
}

impl From<nimiq_block::TendermintProof> for TendermintProof {
    fn from(tendermint_proof: nimiq_block::TendermintProof) -> Self {
        Self {
            round: tendermint_proof.round,
            sig: tendermint_proof.sig,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroJustification {
    signature: CompressedSignature,
    #[serde(skip_serializing_if = "Option::is_none")]
    view_change_proof: Option<ViewChangeProof>,
}

impl From<nimiq_block::MicroJustification> for MicroJustification {
    fn from(justification: nimiq_block::MicroJustification) -> Self {
        Self {
            signature: justification.signature,
            view_change_proof: justification.view_change_proof,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub slot_number: u16,
    pub validator: Address,
    pub public_key: CompressedPublicKey,
}

impl Slot {
    pub fn from(blockchain: &Blockchain, block_number: u32, view_number: u32) -> Self {
        let (validator, slot_number) = blockchain
            .get_slot_owner_at(block_number, view_number, None)
            .expect("Couldn't calculate slot owner!");

        Slot {
            slot_number,
            validator: validator.validator_address.clone(),
            public_key: validator.public_key.compressed().clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slots {
    pub first_slot_number: u16,
    pub num_slots: u16,
    pub validator: Address,
    pub public_key: CompressedPublicKey,
}

impl Slots {
    pub fn from_slots(validators: Validators) -> Vec<Slots> {
        let mut slots = vec![];

        for validator in validators.iter() {
            slots.push(Slots {
                first_slot_number: validator.slot_range.0,
                num_slots: validator.num_slots(),
                validator: validator.validator_address.clone(),
                public_key: validator.public_key.compressed().clone(),
            })
        }

        slots
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashedSlots {
    pub block_number: u32,
    pub lost_rewards: BitSet,
    pub disabled: BitSet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParkedSet {
    pub block_number: u32,
    pub validators: Vec<Address>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkProof {
    pub block_number: u32,
    pub view_number: u32,
    pub hashes: [Blake2bHash; 2],
}

impl From<nimiq_block::ForkProof> for ForkProof {
    fn from(fork_proof: nimiq_block::ForkProof) -> Self {
        let hashes = [fork_proof.header1.hash(), fork_proof.header2.hash()];

        Self {
            block_number: fork_proof.header1.block_number,
            view_number: fork_proof.header1.view_number,
            hashes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub hash: Blake2bHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmations: Option<u32>,

    pub from: Address,
    pub to: Address,
    pub value: Coin,
    pub fee: Coin,
    #[serde(with = "crate::serde_helpers::hex")]
    pub data: Vec<u8>,
    pub flags: u8,
    pub validity_start_height: u32,
    #[serde(with = "crate::serde_helpers::hex")]
    pub proof: Vec<u8>,
}

impl Transaction {
    pub fn from_blockchain(
        transaction: nimiq_transaction::Transaction,
        block_number: u32,
        timestamp: u64,
        head_height: u32,
    ) -> Self {
        Transaction {
            hash: transaction.hash(),
            block_number: Some(block_number),
            timestamp: Some(timestamp),
            confirmations: Some(head_height.saturating_sub(block_number) + 1),
            from: transaction.sender,
            to: transaction.recipient,
            value: transaction.value,
            fee: transaction.fee,
            flags: transaction.flags.bits() as u8,
            data: transaction.data,
            validity_start_height: transaction.validity_start_height,
            proof: transaction.proof,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inherent {
    pub ty: u8,
    pub block_number: u32,
    pub timestamp: u64,
    pub target: Address,
    pub value: Coin,
    #[serde(with = "crate::serde_helpers::hex")]
    pub data: Vec<u8>,
    pub hash: Blake2bHash,
}

impl Inherent {
    pub fn from_transaction(
        inherent: nimiq_account::Inherent,
        block_number: u32,
        timestamp: u64,
    ) -> Self {
        let hash = inherent.hash();

        Inherent {
            ty: inherent.ty as u8,
            block_number,
            timestamp,
            target: inherent.target,
            value: inherent.value,
            data: inherent.data,
            hash,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub address: Address,
    pub balance: Coin,
    #[serde(rename = "type", with = "crate::serde_helpers::account_type")]
    pub ty: AccountType,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub account_additional_fields: Option<AccountAdditionalFields>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AccountAdditionalFields {
    /// Additional account information for vesting contracts.
    VestingContract {
        /// User friendly address (NQ-address) of the owner of the vesting contract.
        owner: Address,
        /// The block that the vesting contracted commenced.
        vesting_start: u64,
        /// The number of blocks after which some part of the vested funds is released.
        vesting_step_blocks: u64,
        /// The amount (in Luna) released every vestingStepBlocks blocks.
        vesting_step_amount: Coin,
        /// The total amount (in smallest unit) that was provided at the contract creation.
        vesting_total_amount: Coin,
    },

    /// Additional account information for HTLC contracts.
    HTLC {
        /// User friendly address (NQ-address) of the sender of the HTLC.
        sender: Address,
        /// User friendly address (NQ-address) of the recipient of the HTLC.
        recipient: Address,
        /// Hex-encoded 32 byte hash root.
        #[serde(with = "serde_with::rust::display_fromstr")]
        hash_root: AnyHash,
        /// Number of hashes this HTLC is split into
        hash_count: u8,
        /// Block after which the contract can only be used by the original sender to recover funds.
        timeout: u64,
        /// The total amount (in smallest unit) that was provided at the contract creation.
        total_amount: Coin,
    },
}

impl Account {
    pub fn from_account(address: Address, account: nimiq_account::Account) -> Self {
        match account {
            nimiq_account::Account::Basic(basic) => Account {
                address,
                balance: basic.balance,
                ty: AccountType::Basic,
                account_additional_fields: None,
            },
            nimiq_account::Account::Vesting(vesting) => Account {
                address,
                balance: vesting.balance,
                ty: AccountType::Vesting,
                account_additional_fields: Some(AccountAdditionalFields::VestingContract {
                    owner: vesting.owner.clone(),
                    vesting_start: vesting.start_time,
                    vesting_step_blocks: vesting.time_step,
                    vesting_step_amount: vesting.step_amount,
                    vesting_total_amount: vesting.total_amount,
                }),
            },
            nimiq_account::Account::HTLC(htlc) => Account {
                address,
                balance: htlc.balance,
                ty: AccountType::Vesting,
                account_additional_fields: Some(AccountAdditionalFields::HTLC {
                    sender: htlc.sender.clone(),
                    recipient: htlc.recipient.clone(),
                    hash_root: htlc.hash_root,
                    hash_count: htlc.hash_count,
                    timeout: htlc.timeout,
                    total_amount: htlc.total_amount,
                }),
            },
            nimiq_account::Account::Staking(staking) => Account {
                address,
                balance: staking.balance,
                ty: AccountType::Staking,
                account_additional_fields: None,
            },
            _ => unreachable!(),
        }
    }

    pub fn empty(address: Address) -> Self {
        Account {
            address,
            balance: Coin::ZERO,
            ty: AccountType::Basic,
            account_additional_fields: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Staker {
    pub address: Address,
    pub active_stake: Coin,
    pub inactive_stake: Coin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Address>,
    pub retire_time: u32,
}

impl Staker {
    pub fn from_staker(staker: &nimiq_account::Staker) -> Self {
        Staker {
            address: staker.address.clone(),
            active_stake: staker.active_stake,
            inactive_stake: staker.inactive_stake,
            delegation: staker.delegation.clone(),
            retire_time: staker.retire_time,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Validator {
    pub address: Address,
    pub warm_key: Address,
    pub validator_key: CompressedPublicKey,
    pub reward_address: Address,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_data: Option<Blake2bHash>,
    pub balance: Coin,
    pub num_stakers: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactivity_flag: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stakers: Option<HashMap<Address, Coin>>,
}

impl Validator {
    pub fn from_validator(
        validator: &nimiq_account::Validator,
        stakers: Option<HashMap<Address, Coin>>,
    ) -> Self {
        Validator {
            address: validator.address.clone(),
            warm_key: validator.warm_key.clone(),
            validator_key: validator.validator_key.clone(),
            reward_address: validator.reward_address.clone(),
            signal_data: validator.signal_data.clone(),
            balance: validator.balance,
            num_stakers: validator.num_stakers,
            inactivity_flag: validator.inactivity_flag,
            stakers,
        }
    }
}
