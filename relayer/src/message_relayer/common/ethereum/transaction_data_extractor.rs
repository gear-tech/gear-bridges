use crate::{
    hex_utils,
    message_relayer::common::{web_request::EthTransaction, EthereumSlotNumber, TxHashWithSlot},
};
use alloy::sol_types::SolEvent;
use anyhow::Context;
use ethereum_client::{abi::BridgingPayment, inclusion::RecordedReceiptProof, PollingEthApi};
use ethereum_common::SECONDS_PER_SLOT;
use primitive_types::H160;
use tokio::sync::mpsc::{Receiver, WeakSender};

/// Authenticates finalized paid receipts before forwarding HTTP requests to the tx manager.
pub struct TransactionDataExtractor {
    eth_api: PollingEthApi,
    genesis_time: u64,
    bridging_payment_address: H160,
    sender: WeakSender<TxHashWithSlot>,
    receiver: Receiver<EthTransaction>,
    minimum_receipt_slot: u64,
    last_receipt_slot: Option<u64>,
}

impl TransactionDataExtractor {
    pub fn new(
        eth_api: PollingEthApi,
        genesis_time: u64,
        bridging_payment_address: H160,
        sender: WeakSender<TxHashWithSlot>,
        receiver: Receiver<EthTransaction>,
    ) -> Self {
        Self {
            eth_api,
            genesis_time,
            bridging_payment_address,
            sender,
            receiver,
            minimum_receipt_slot: 0,
            last_receipt_slot: None,
        }
    }

    pub fn with_receipt_domain(mut self, minimum: u64, last: Option<u64>) -> Self {
        self.minimum_receipt_slot = minimum;
        self.last_receipt_slot = last;
        self
    }

    pub fn spawn(self) {
        tokio::task::spawn(task(self));
    }
}

async fn task(mut this: TransactionDataExtractor) {
    while let Some(request) = this.receiver.recv().await {
        let Some(sender) = this.sender.upgrade() else {
            return;
        };
        let tx = match authenticate_request(&mut this, &request).await {
            Ok(tx) => tx,
            Err(error) => {
                // A bad operator request must not wedge subsequent requests or discovery.
                log::warn!("HTTP transaction rejected before proof admission: {error:#}");
                continue;
            }
        };
        if sender.send(tx).await.is_err() {
            return;
        }
    }
}

async fn authenticate_request(
    this: &mut TransactionDataExtractor,
    request: &EthTransaction,
) -> anyhow::Result<TxHashWithSlot> {
    let tx_hash = hex_utils::decode_h256(&request.tx_hash)
        .context("Failed to decode transaction hash")?
        .0
        .into();
    let evidence = crate::rpc::retry_polling_eth(
        &mut this.eth_api,
        "HTTP paid finalized receipt",
        |api| async move { api.get_finalized_receipt(tx_hash).await },
    )
    .await?;
    let slot_number = paid_receipt_slot(
        evidence.as_ref().map(|receipt| &receipt.inclusion),
        this.bridging_payment_address,
        this.genesis_time,
        this.minimum_receipt_slot,
        this.last_receipt_slot,
    )?;
    Ok(TxHashWithSlot {
        slot_number,
        tx_hash,
    })
}

// Only called with the inclusion returned by get_finalized_receipt, never RPC log hints.
fn paid_receipt_slot(
    inclusion: Option<&RecordedReceiptProof>,
    payment: H160,
    genesis_time: u64,
    minimum: u64,
    last: Option<u64>,
) -> anyhow::Result<EthereumSlotNumber> {
    let inclusion = inclusion.context("HOLD: HTTP transaction is not canonically finalized")?;
    anyhow::ensure!(
        inclusion.receipt.status(),
        "HOLD: HTTP transaction reverted"
    );
    let mut paid = false;
    for log in inclusion.receipt.logs() {
        if log.address.as_slice() == payment.as_bytes()
            && log.topics().first() == Some(&BridgingPayment::FeePaid::SIGNATURE_HASH)
        {
            BridgingPayment::FeePaid::decode_log_validate(log)
                .context("HOLD: malformed authenticated FeePaid")?;
            paid = true;
        }
    }
    anyhow::ensure!(
        paid,
        "HOLD: HTTP receipt has no FeePaid from the configured payment contract"
    );
    let elapsed = inclusion
        .header
        .timestamp
        .checked_sub(genesis_time)
        .context("HOLD: Ethereum timestamp precedes Beacon genesis")?;
    anyhow::ensure!(
        elapsed % SECONDS_PER_SLOT == 0,
        "HOLD: Ethereum timestamp does not identify a Beacon slot"
    );
    let slot = elapsed / SECONDS_PER_SLOT;
    anyhow::ensure!(
        slot >= minimum && last.is_none_or(|last| slot <= last),
        "HOLD: HTTP receipt is outside this receiver deployment domain"
    );
    Ok(EthereumSlotNumber(slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{Receipt, ReceiptEnvelope},
        primitives::{Address, Log, B256},
    };

    // Reconstructed from supplied PR860 HTML, O3-04; not the unavailable original PoC.
    #[test]
    fn reconstructed_paid_http_requires_finalized_exact_emitter_and_authenticated_slot() {
        let payment = H160::repeat_byte(3);
        let fee = Log {
            address: Address::from_slice(payment.as_bytes()),
            data: BridgingPayment::FeePaid {}.encode_log_data(),
        };
        let mut proof = RecordedReceiptProof {
            genesis: B256::ZERO,
            header: Default::default(),
            index: 0,
            raw_transaction: Vec::new(),
            receipt: ReceiptEnvelope::Legacy(
                Receipt {
                    status: true.into(),
                    cumulative_gas_used: 21_000,
                    logs: vec![fee.clone()],
                }
                .with_bloom(),
            ),
            transaction_proof: Vec::new(),
            receipt_proof: Vec::new(),
        };
        proof.header.timestamp = 1_120;
        assert!(paid_receipt_slot(None, payment, 1_000, 0, None).is_err());
        assert!(paid_receipt_slot(Some(&proof), H160::repeat_byte(4), 1_000, 0, None).is_err());
        assert_eq!(
            paid_receipt_slot(Some(&proof), payment, 1_000, 10, Some(10))
                .unwrap()
                .0,
            10
        );
        assert!(paid_receipt_slot(Some(&proof), payment, 1_000, 11, None).is_err());
        assert!(paid_receipt_slot(Some(&proof), payment, 1_000, 0, Some(9)).is_err());
        proof.header.timestamp += 1;
        assert!(paid_receipt_slot(Some(&proof), payment, 1_000, 0, None).is_err());
        proof.header.timestamp = 1_120;
        for (success, logs) in [(true, vec![]), (false, vec![fee])] {
            proof.receipt = ReceiptEnvelope::Legacy(
                Receipt {
                    status: success.into(),
                    cumulative_gas_used: 21_000,
                    logs,
                }
                .with_bloom(),
            );
            assert!(paid_receipt_slot(Some(&proof), payment, 1_000, 0, None).is_err());
        }
    }
}
