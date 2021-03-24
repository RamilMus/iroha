//! This module contains Transaction related functionality of the Iroha.
//!
//! `Transaction` is the start of the Transaction lifecycle.

use std::{
    cmp::min,
    time::{Duration, SystemTime},
};

pub use iroha_data_model::prelude::*;
use iroha_derive::Io;
use iroha_error::{error, Result, WrapErr};
use iroha_version::{declare_versioned_with_scale, version_with_scale};
use parity_scale_codec::{Decode, Encode};

use crate::{expression::Evaluate, isi::Execute, permissions::PermissionsValidatorBox, prelude::*};

declare_versioned_with_scale!(VersionedAcceptedTransaction 1..2);

#[allow(clippy::missing_errors_doc)]
impl VersionedAcceptedTransaction {
    /// Same as [`as_v1`] but also does conversion
    pub const fn as_inner_v1(&self) -> &AcceptedTransaction {
        match self {
            VersionedAcceptedTransaction::V1(v1) => &v1.0,
        }
    }

    /// Same as [`as_inner_v1`] but returns mutable reference
    pub fn as_mut_inner_v1(&mut self) -> &mut AcceptedTransaction {
        match self {
            VersionedAcceptedTransaction::V1(v1) => &mut v1.0,
        }
    }

    /// Same as [`into_v1`] but also does conversion
    #[allow(clippy::missing_const_for_fn)]
    pub fn into_inner_v1(self) -> AcceptedTransaction {
        match self {
            VersionedAcceptedTransaction::V1(v1) => v1.0,
        }
    }

    /// Accepts transaction
    pub fn from_transaction(
        transaction: Transaction,
        max_instruction_number: usize,
    ) -> Result<VersionedAcceptedTransaction> {
        AcceptedTransaction::from_transaction(transaction, max_instruction_number).map(Into::into)
    }

    /// Calculate transaction `Hash`.
    pub fn hash(&self) -> Hash {
        self.as_inner_v1().hash()
    }

    /// Checks if this transaction is waiting longer than specified in `transaction_time_to_live` from `QueueConfiguration` or `time_to_live_ms` of this transaction.
    /// Meaning that the transaction will be expired as soon as the lesser of the specified TTLs was reached.
    pub fn is_expired(&self, transaction_time_to_live: Duration) -> bool {
        self.as_inner_v1().is_expired(transaction_time_to_live)
    }

    /// Move transaction lifecycle forward by checking an ability to apply instructions to the
    /// `WorldStateView`.
    ///
    /// Returns `Ok(ValidTransaction)` if succeeded and `Err(String)` if failed.
    pub fn validate(
        self,
        wsv: &WorldStateView,
        permissions_validator: &PermissionsValidatorBox,
        is_genesis: bool,
    ) -> Result<VersionedValidTransaction, VersionedRejectedTransaction> {
        self.into_inner_v1()
            .validate(wsv, permissions_validator, is_genesis)
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Checks that the signatures of this transaction satisfy the signature condition specified in the account.
    pub fn check_signature_condition(&self, wsv: &WorldStateView) -> Result<bool> {
        self.as_inner_v1().check_signature_condition(wsv)
    }

    /// Rejects transaction with the `rejection_reason`.
    pub fn reject(
        self,
        rejection_reason: TransactionRejectionReason,
    ) -> VersionedRejectedTransaction {
        self.into_inner_v1().reject(rejection_reason).into()
    }

    /// Checks if this transaction has already been committed or rejected.
    pub fn is_in_blockchain(&self, wsv: &WorldStateView) -> bool {
        self.as_inner_v1().is_in_blockchain(wsv)
    }

    /// # Errors
    /// Asserts specific instruction number of instruction in transaction constraint
    pub fn check_instruction_len(&self, max_instruction_len: usize) -> Result<()> {
        self.as_inner_v1()
            .check_instruction_len(max_instruction_len)
    }
}

/// `AcceptedTransaction` represents a transaction accepted by iroha peer.
#[version_with_scale(n = 1, versioned = "VersionedAcceptedTransaction")]
#[derive(Clone, Debug, Io, Encode, Decode)]
pub struct AcceptedTransaction {
    /// Payload of this transaction.
    pub payload: Payload,
    /// Signatures for this transaction.
    pub signatures: Vec<Signature>,
}

impl AcceptedTransaction {
    /// # Errors
    /// Asserts specific instruction number of instruction in transaction constraint
    pub fn check_instruction_len(&self, max_instruction_len: usize) -> Result<()> {
        self.payload.check_instruction_len(max_instruction_len)
    }

    /// Accepts transaction
    ///
    /// # Errors
    /// Can fail if verification of some signature fails
    pub fn from_transaction(
        transaction: Transaction,
        max_instruction_number: usize,
    ) -> Result<AcceptedTransaction> {
        transaction
            .check_instruction_len(max_instruction_number)
            .wrap_err("Failed to accept transaction")?;

        for signature in &transaction.signatures {
            signature
                .verify(transaction.hash().as_ref())
                .wrap_err("Failed to verify signatures")?;
        }

        Ok(Self {
            payload: transaction.payload,
            signatures: transaction.signatures,
        })
    }

    /// Calculate transaction `Hash`.
    pub fn hash(&self) -> Hash {
        let bytes: Vec<u8> = self.payload.clone().into();
        Hash::new(&bytes)
    }

    /// Checks if this transaction is waiting longer than specified in `transaction_time_to_live` from `QueueConfiguration` or `time_to_live_ms` of this transaction.
    /// Meaning that the transaction will be expired as soon as the lesser of the specified TTLs was reached.
    pub fn is_expired(&self, transaction_time_to_live: Duration) -> bool {
        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Failed to get System Time.");

        (current_time - Duration::from_millis(self.payload.creation_time))
            > min(
                Duration::from_millis(self.payload.time_to_live_ms),
                transaction_time_to_live,
            )
    }

    fn validate_internal(
        &self,
        world_state_view: &WorldStateView,
        permissions_validator: &PermissionsValidatorBox,
        is_genesis: bool,
    ) -> Result<(), TransactionRejectionReason> {
        let mut world_state_view_temp = world_state_view.clone();
        let account_id = self.payload.account_id.clone();
        if !is_genesis && account_id == <Account as Identifiable>::Id::genesis_account() {
            return Err(TransactionRejectionReason::UnexpectedGenesisAccountSignature);
        }

        drop(
            self.signatures
                .iter()
                .map(|signature| {
                    signature.verify(self.hash().as_ref()).map_err(|reason| {
                        SignatureVerificationFail {
                            signature: signature.clone(),
                            // TODO: Should here also be iroha_error::Error?
                            reason: reason.to_string(),
                        }
                    })
                })
                .collect::<Result<Vec<()>, _>>()
                .map_err(TransactionRejectionReason::SignatureVerification)?,
        );

        let option_reason = match self.check_signature_condition(world_state_view) {
            Ok(true) => None,
            Ok(false) => Some("Signature condition not satisfied.".to_owned()),
            Err(reason) => Some(reason.to_string()),
        }
        .map(|reason| UnsatisfiedSignatureConditionFail { reason })
        .map(TransactionRejectionReason::UnsatisfiedSignatureCondition);

        if let Some(reason) = option_reason {
            return Err(reason);
        }

        for instruction in &self.payload.instructions {
            let account_id = self.payload.account_id.clone();

            world_state_view_temp = instruction
                .clone()
                .execute(account_id.clone(), &world_state_view_temp)
                .map_err(|reason| InstructionExecutionFail {
                    instruction: instruction.clone(),
                    reason: reason.to_string(),
                })
                .map_err(TransactionRejectionReason::InstructionExecution)?;

            if !is_genesis {
                permissions_validator
                    .check_instruction(account_id.clone(), instruction.clone(), world_state_view)
                    .map_err(|reason| NotPermittedFail { reason })
                    .map_err(TransactionRejectionReason::NotPermitted)?;
            }
        }

        Ok(())
    }

    /// Move transaction lifecycle forward by checking an ability to apply instructions to the
    /// `WorldStateView`.
    ///
    /// # Errors
    /// Can fail if:
    /// - signature verification fails
    /// - instruction execution fails
    /// - permission check fails
    pub fn validate(
        self,
        world_state_view: &WorldStateView,
        permissions_validator: &PermissionsValidatorBox,
        is_genesis: bool,
    ) -> Result<ValidTransaction, RejectedTransaction> {
        match self.validate_internal(world_state_view, permissions_validator, is_genesis) {
            Ok(()) => Ok(ValidTransaction {
                payload: self.payload,
                signatures: self.signatures,
            }),
            Err(reason) => Err(self.reject(reason)),
        }
    }

    /// Checks that the signatures of this transaction satisfy the signature condition specified in the account.
    ///
    /// # Errors
    /// Can fail if signature conditionon account fails or if account is not found
    pub fn check_signature_condition(&self, world_state_view: &WorldStateView) -> Result<bool> {
        let account_id = self.payload.account_id.clone();
        world_state_view
            .read_account(&account_id)
            .ok_or_else(|| error!("Account with id {} not found", account_id))?
            .check_signature_condition(&self.signatures)
            .evaluate(world_state_view, &Context::new())
    }

    /// Rejects transaction with the `rejection_reason`.
    #[allow(clippy::missing_const_for_fn)]
    pub fn reject(self, rejection_reason: TransactionRejectionReason) -> RejectedTransaction {
        RejectedTransaction {
            payload: self.payload,
            signatures: self.signatures,
            rejection_reason,
        }
    }

    /// Checks if this transaction has already been committed or rejected.
    pub fn is_in_blockchain(&self, world_state_view: &WorldStateView) -> bool {
        world_state_view.has_transaction(self.hash())
    }
}

impl From<VersionedAcceptedTransaction> for VersionedTransaction {
    fn from(tx: VersionedAcceptedTransaction) -> Self {
        let tx: AcceptedTransaction = tx.into_inner_v1();
        let tx: Transaction = tx.into();
        tx.into()
    }
}

impl From<AcceptedTransaction> for Transaction {
    fn from(transaction: AcceptedTransaction) -> Self {
        Transaction {
            payload: transaction.payload,
            signatures: transaction.signatures,
        }
    }
}

impl From<VersionedValidTransaction> for VersionedTransaction {
    fn from(transaction: VersionedValidTransaction) -> Self {
        match transaction {
            VersionedValidTransaction::V1(v1) => {
                let transaction: ValidTransaction = v1.0;
                VersionedTransaction::new(
                    transaction.payload.instructions,
                    transaction.payload.account_id,
                    transaction.payload.time_to_live_ms,
                )
            }
        }
    }
}

impl IsInBlockchain for VersionedRejectedTransaction {
    fn is_in_blockchain(&self, world_state_view: &WorldStateView) -> bool {
        self.as_inner_v1().is_in_blockchain(world_state_view)
    }
}

impl IsInBlockchain for RejectedTransaction {
    fn is_in_blockchain(&self, world_state_view: &WorldStateView) -> bool {
        world_state_view.has_transaction(self.hash())
    }
}

declare_versioned_with_scale!(VersionedValidTransaction 1..2);

#[allow(clippy::missing_errors_doc)]
impl VersionedValidTransaction {
    /// Same as [`as_v1`] but also does conversion
    pub const fn as_inner_v1(&self) -> &ValidTransaction {
        match self {
            Self::V1(v1) => &v1.0,
        }
    }

    /// Same as [`as_inner_v1`] but returns mutable reference
    pub fn as_mut_inner_v1(&mut self) -> &mut ValidTransaction {
        match self {
            Self::V1(v1) => &mut v1.0,
        }
    }

    /// Same as [`into_v1`] but also does conversion
    #[allow(clippy::missing_const_for_fn)]
    pub fn into_inner_v1(self) -> ValidTransaction {
        match self {
            Self::V1(v1) => v1.0,
        }
    }

    /// Apply instructions to the `WorldStateView`.
    pub fn proceed(&self, wsv: &mut WorldStateView) -> Result<()> {
        self.as_inner_v1().proceed(wsv)
    }

    /// Calculate transaction `Hash`.
    pub fn hash(&self) -> Hash {
        self.as_inner_v1().hash()
    }

    /// Checks if this transaction has already been committed or rejected.
    pub fn is_in_blockchain(&self, wsv: &WorldStateView) -> bool {
        self.as_inner_v1().is_in_blockchain(wsv)
    }

    /// # Errors
    /// Asserts specific instruction number of instruction in transaction constraint
    pub fn check_instruction_len(&self, max_instruction_len: usize) -> Result<()> {
        self.as_inner_v1()
            .check_instruction_len(max_instruction_len)
    }
}

/// `ValidTransaction` represents trustfull Transaction state.
#[version_with_scale(n = 1, versioned = "VersionedValidTransaction")]
#[derive(Clone, Debug, Io, Encode, Decode)]
pub struct ValidTransaction {
    payload: Payload,
    signatures: Vec<Signature>,
}

impl ValidTransaction {
    /// # Errors
    /// Asserts specific instruction number of instruction in transaction constraint
    pub fn check_instruction_len(&self, max_instruction_len: usize) -> Result<()> {
        self.payload.check_instruction_len(max_instruction_len)
    }

    /// Apply instructions to the `WorldStateView`.
    ///
    /// # Errors
    /// Can fail if execution of instructions fail
    pub fn proceed(&self, world_state_view: &mut WorldStateView) -> Result<()> {
        let mut world_state_view_temp = world_state_view.clone();
        for instruction in &self.payload.instructions {
            world_state_view_temp = instruction
                .clone()
                .execute(self.payload.account_id.clone(), &world_state_view_temp)?;
        }
        *world_state_view = world_state_view_temp;
        Ok(())
    }

    /// Calculate transaction `Hash`.
    pub fn hash(&self) -> Hash {
        let bytes: Vec<u8> = self.payload.clone().into();
        Hash::new(&bytes)
    }

    /// Checks if this transaction has already been committed or rejected.
    pub fn is_in_blockchain(&self, world_state_view: &WorldStateView) -> bool {
        world_state_view.has_transaction(self.hash())
    }
}

impl From<VersionedValidTransaction> for VersionedAcceptedTransaction {
    fn from(tx: VersionedValidTransaction) -> Self {
        let tx: ValidTransaction = tx.into_inner_v1();
        let tx: AcceptedTransaction = tx.into();
        tx.into()
    }
}

impl From<ValidTransaction> for AcceptedTransaction {
    fn from(transaction: ValidTransaction) -> Self {
        AcceptedTransaction {
            payload: transaction.payload,
            signatures: transaction.signatures,
        }
    }
}

impl From<VersionedRejectedTransaction> for VersionedAcceptedTransaction {
    fn from(tx: VersionedRejectedTransaction) -> Self {
        let tx: RejectedTransaction = tx.into_inner_v1();
        let tx: AcceptedTransaction = tx.into();
        tx.into()
    }
}

impl From<RejectedTransaction> for AcceptedTransaction {
    fn from(transaction: RejectedTransaction) -> Self {
        AcceptedTransaction {
            payload: transaction.payload,
            signatures: transaction.signatures,
        }
    }
}

/// Query module provides [`IrohaQuery`] Transaction related implementations.
pub mod query {
    use iroha_data_model::prelude::*;
    use iroha_derive::*;
    use iroha_error::Result;

    use super::*;

    impl Query for FindTransactionsByAccountId {
        #[log]
        fn execute(&self, world_state_view: &WorldStateView) -> Result<Value> {
            let id = self
                .account_id
                .evaluate(world_state_view, &Context::default())
                .wrap_err("Failed to get id")?;
            Ok(Value::Vec(
                world_state_view
                    .read_transactions(&id)
                    .into_iter()
                    .cloned()
                    .map(Value::TransactionValue)
                    .collect::<Vec<_>>(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::default_trait_access)]

    use iroha_data_model::{
        account::GENESIS_ACCOUNT_NAME, domain::GENESIS_DOMAIN_NAME,
        transaction::MAX_INSTRUCTION_NUMBER,
    };
    use iroha_error::{Error, MessageError, Result, WrappedError};

    use super::*;
    use crate::{config::Configuration, init, permissions::AllowAll};

    const CONFIGURATION_PATH: &str = "tests/test_config.json";

    #[test]
    fn hash_should_be_the_same() {
        let key_pair = &KeyPair::generate().expect("Failed to generate key pair.");

        let mut config =
            Configuration::from_path(CONFIGURATION_PATH).expect("Failed to load configuration.");
        config.genesis_configuration.genesis_account_private_key =
            Some(key_pair.private_key.clone());
        config.genesis_configuration.genesis_account_public_key = key_pair.public_key.clone();

        let tx = Transaction::new(
            vec![],
            AccountId::new(GENESIS_ACCOUNT_NAME, GENESIS_DOMAIN_NAME),
            1000,
        );
        let tx_hash = tx.hash();

        let signed_tx = tx.sign(key_pair).expect("Failed to sign.");
        let signed_tx_hash = signed_tx.hash();
        let accepted_tx =
            AcceptedTransaction::from_transaction(signed_tx, 4096).expect("Failed to accept.");
        let accepted_tx_hash = accepted_tx.hash();
        let valid_tx_hash = accepted_tx
            .validate(
                &WorldStateView::new(World::with(init::domains(&config), Default::default())),
                &AllowAll.into(),
                true,
            )
            .expect("Failed to validate.")
            .hash();
        assert_eq!(tx_hash, signed_tx_hash);
        assert_eq!(tx_hash, accepted_tx_hash);
        assert_eq!(tx_hash, valid_tx_hash);
    }

    #[test]
    fn transaction_not_accepted() {
        let inst = FailBox {
            message: "Will fail".to_owned(),
        };
        let tx = Transaction::new(
            vec![inst.into(); MAX_INSTRUCTION_NUMBER + 1],
            AccountId::new("root", "global"),
            1000,
        );
        let result: Result<AcceptedTransaction> = AcceptedTransaction::from_transaction(tx, 4096);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err = err
            .downcast_ref::<WrappedError<&'static str, Error>>()
            .unwrap();
        assert_eq!(err.msg, "Failed to accept transaction");
        let err = err.downcast_ref::<MessageError<&'static str>>().unwrap();
        assert_eq!(err.msg, "Too many instructions in payload");
    }
}