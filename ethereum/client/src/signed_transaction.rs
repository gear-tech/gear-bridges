//! Strict signed-envelope mechanics; callers retain their lane and calldata policy.
use crate::TransactionIdentity;
use alloy::{
    consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope},
    eips::Decodable2718,
    primitives::{keccak256, Address, B256},
};
use anyhow::{ensure, Context, Result};

pub struct SignedTransactionExpected {
    pub hash: B256,
    pub nonce: u64,
    pub destination: Address,
    pub chain_id: Option<u64>,
    pub signer: Option<Address>,
}

/// Diagnostics and split/grouped trailing-byte checks preserve each caller's admission order.
pub struct SignedTransactionErrors {
    pub hash: &'static str,
    pub decode: &'static str,
    pub trailing: Option<&'static str>,
    pub identity: &'static str,
    pub signature: &'static str,
    pub signer: &'static str,
}

pub fn decode_signed_transaction(
    raw: &[u8],
    expected: SignedTransactionExpected,
    errors: SignedTransactionErrors,
) -> Result<(TxEnvelope, TransactionIdentity)> {
    ensure!(keccak256(raw) == expected.hash, errors.hash);
    let mut encoded = raw;
    let transaction = TxEnvelope::decode_2718(&mut encoded).context(errors.decode)?;
    if let Some(error) = errors.trailing {
        ensure!(encoded.is_empty(), error);
    }
    ensure!(
        encoded.is_empty()
            && transaction.nonce() == expected.nonce
            && transaction.to() == Some(expected.destination)
            && expected
                .chain_id
                .is_none_or(|chain| transaction.chain_id() == Some(chain)),
        errors.identity
    );
    let from = transaction.recover_signer().context(errors.signature)?;
    ensure!(
        expected.signer.is_none_or(|signer| from == signer),
        errors.signer
    );
    let identity = TransactionIdentity {
        hash: expected.hash,
        from,
        nonce: expected.nonce,
        to: Some(expected.destination),
        block_number: None,
    };
    Ok((transaction, identity))
}
