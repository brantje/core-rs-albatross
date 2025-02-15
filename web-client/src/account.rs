use tsify::Tsify;
use wasm_bindgen::prelude::*;

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainBasicAccount {
    balance: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainVestingContract {
    balance: u64,
    owner: String,
    start_time: u64,
    time_step: u64,
    step_amount: u64,
    total_amount: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcContract {
    balance: u64,
    sender: String,
    recipient: String,
    hash_algorithm: String,
    hash_root: String,
    hash_count: u8,
    timeout: u64,
    total_amount: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainStakingContract {
    balance: u64,
    active_validators: Vec<(String, u64)>,
    parked_set: Vec<String>,
    current_lost_rewards: String,
    previous_lost_rewards: String,
    current_disabled_slots: Vec<(String, Vec<u16>)>,
    previous_disabled_slots: Vec<(String, Vec<u16>)>,
}

#[derive(serde::Serialize, Tsify)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlainAccount {
    Basic(PlainBasicAccount),
    Vesting(PlainVestingContract),
    Htlc(PlainHtlcContract),
    Staking(PlainStakingContract),
}

impl PlainAccount {
    pub fn from_native(account: &nimiq_account::Account) -> PlainAccount {
        match account {
            nimiq_account::Account::Basic(acc) => PlainAccount::Basic(PlainBasicAccount {
                balance: acc.balance.into(),
            }),
            nimiq_account::Account::Vesting(acc) => PlainAccount::Vesting(PlainVestingContract {
                balance: acc.balance.into(),
                owner: acc.owner.to_user_friendly_address(),
                start_time: acc.start_time,
                time_step: acc.time_step,
                step_amount: acc.step_amount.into(),
                total_amount: acc.total_amount.into(),
            }),
            nimiq_account::Account::HTLC(acc) => PlainAccount::Htlc(PlainHtlcContract {
                balance: acc.balance.into(),
                sender: acc.sender.to_user_friendly_address(),
                recipient: acc.recipient.to_user_friendly_address(),
                hash_algorithm: match acc.hash_algorithm {
                    nimiq_transaction::account::htlc_contract::HashAlgorithm::Blake2b => {
                        "blake2b".to_string()
                    }
                    nimiq_transaction::account::htlc_contract::HashAlgorithm::Sha256 => {
                        "sha256".to_string()
                    }
                },
                hash_root: acc.hash_root.to_hex(),
                hash_count: acc.hash_count,
                timeout: acc.timeout,
                total_amount: acc.total_amount.into(),
            }),
            nimiq_account::Account::Staking(acc) => PlainAccount::Staking(PlainStakingContract {
                balance: acc.balance.into(),
                active_validators: acc
                    .active_validators
                    .iter()
                    .map(|(address, balance)| {
                        (
                            address.to_user_friendly_address(),
                            balance.to_owned().into(),
                        )
                    })
                    .collect(),
                parked_set: acc
                    .parked_set
                    .iter()
                    .map(|address| address.to_user_friendly_address())
                    .collect(),
                current_lost_rewards: acc.current_lost_rewards.to_string(),
                previous_lost_rewards: acc.previous_lost_rewards.to_string(),
                current_disabled_slots: acc
                    .current_disabled_slots
                    .iter()
                    .map(|(address, slots)| {
                        (
                            address.to_user_friendly_address(),
                            slots.iter().cloned().collect(),
                        )
                    })
                    .collect(),
                previous_disabled_slots: acc
                    .previous_disabled_slots
                    .iter()
                    .map(|(address, slots)| {
                        (
                            address.to_user_friendly_address(),
                            slots.iter().cloned().collect(),
                        )
                    })
                    .collect(),
            }),
        }
    }
}

/// JSON-compatible and human-readable format of a staker. E.g. delegation addresses are presented in their
/// human-readable format.
#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainStaker {
    /// The staker's balance.
    balance: u64,
    /// The address of the validator for which the staker is delegating its stake for. If it is not
    /// delegating to any validator, this will be set to None.
    delegation: Option<String>,
}

impl PlainStaker {
    pub fn from_native(staker: &nimiq_account::Staker) -> PlainStaker {
        PlainStaker {
            delegation: staker
                .delegation
                .as_ref()
                .map(|address| address.to_user_friendly_address()),
            balance: staker.balance.into(),
        }
    }
}

/// JSON-compatible and human-readable format of a validator. E.g. reward addresses and public keys are presented in
/// their human-readable format.
#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainValidator {
    /// The public key used to sign blocks. It is also used to retire, reactivate and unpark the validator.
    pub signing_public_key: String,
    /// The voting public key, it is used to vote for skip and macro blocks.
    pub voting_public_key: String,
    /// The reward address of the validator. All the block rewards are paid to this address.
    pub reward_address: String,
    /// Signaling field. Can be used to do chain upgrades or for any other purpose that requires
    /// validators to coordinate among themselves.
    pub signal_data: Option<String>,
    /// The total stake assigned to this validator. It includes the validator deposit as well as the
    /// coins delegated to him by stakers.
    pub total_stake: u64,
    /// The amount of coins deposited by this validator. The initial deposit is a fixed amount,
    /// however this value can be decremented by failing staking transactions due to fees.
    pub deposit: u64,
    /// The number of stakers that are delegating to this validator.
    pub num_stakers: u64,
    /// An option indicating if the validator is inactive. If it is inactive, then it contains the
    /// block height at which it became inactive.
    pub inactive_since: Option<u32>,
    /// A flag indicating if the validator is retired.
    pub retired: bool,
}

impl PlainValidator {
    pub fn from_native(validator: &nimiq_account::Validator) -> PlainValidator {
        PlainValidator {
            signing_public_key: validator.signing_key.to_hex(),
            voting_public_key: validator.voting_key.to_hex(),
            reward_address: validator.reward_address.to_user_friendly_address(),
            signal_data: validator.signal_data.as_ref().map(|data| data.to_hex()),
            total_stake: validator.total_stake.into(),
            deposit: validator.deposit.into(),
            num_stakers: validator.num_stakers,
            inactive_since: validator.inactive_since,
            retired: validator.retired,
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainAccount")]
    pub type PlainAccountType;

    #[wasm_bindgen(typescript_type = "PlainAccount[]")]
    pub type PlainAccountArrayType;

    #[wasm_bindgen(typescript_type = "PlainStaker | undefined")]
    pub type PlainStakerType;

    #[wasm_bindgen(typescript_type = "(PlainStaker | undefined)[]")]
    pub type PlainStakerArrayType;

    #[wasm_bindgen(typescript_type = "PlainValidator | undefined")]
    pub type PlainValidatorType;

    #[wasm_bindgen(typescript_type = "(PlainValidator | undefined)[]")]
    pub type PlainValidatorArrayType;
}
