use log::error;

use nimiq_primitives::account::AccountType;

use crate::account::AccountTransactionVerification;
use crate::{Transaction, TransactionError, TransactionFlags};

pub use self::structs::*;

pub mod structs;

pub struct StakingContractVerifier {}

impl AccountTransactionVerification for StakingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Staking);

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            error!(
                "Contract creation not allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        // Incoming transactions require the data field to be set correctly
        // and we perform static signature checks here.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        if data.is_signaling() != transaction.flags.contains(TransactionFlags::SIGNALING) {
            error!("Signaling must be set for signaling transactions. The offending transaction is the following:\n{:?}", transaction);
            return Err(TransactionError::InvalidForRecipient);
        }

        if data.is_signaling() && !transaction.value.is_zero() {
            error!("Signaling transactions must have a value of zero. The offending transaction is the following:\n{:?}", transaction);
            return Err(TransactionError::InvalidValue);
        }

        data.verify(transaction)?;

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Staking);

        let proof = OutgoingStakingTransactionProof::parse(transaction)?;

        proof.verify(transaction)?;

        Ok(())
    }
}
