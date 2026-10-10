use super::{
    message_sender::{self, MessageSenderIo},
    proof_composer::{self, ProofComposerIo},
    storage::Storage,
};
use crate::message_relayer::{common::TxHashWithSlot, eth_to_gear::message_sender::MessageStatus};
use eth_events_electra_client::EthToVaraEvent;
use prometheus::IntCounter;
use sails_rs::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Instant,
};
use tokio::{
    sync::{mpsc::UnboundedReceiver, RwLock},
    time::{self, Duration},
};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

const CAPACITY: usize = 1_000;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub uuid: Uuid,
    pub status: TxStatus,
    pub tx: TxHashWithSlot,
    pub receipt_observation: Option<message_sender::FinalizedReceiptObservation>,
    pub receipt: Option<ReceiptEvidence>,
}

impl Transaction {
    pub fn new(tx: TxHashWithSlot, status: TxStatus) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            status,
            tx,
            receipt_observation: None,
            receipt: None,
        }
    }

    pub(super) fn receipt_event(&self) -> anyhow::Result<EthToVaraEvent> {
        let receipt = self.receipt.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "HOLD: transaction {} has no original receipt evidence",
                self.uuid
            )
        })?;
        let mut bytes = receipt.payload.as_slice();
        let event = EthToVaraEvent::decode(&mut bytes)?;
        anyhow::ensure!(
            bytes.is_empty()
                && receipt.receipt_key == (event.proof_block.block.slot, event.transaction_index)
                && self.tx.slot_number.0 == event.proof_block.block.slot,
            "HOLD: transaction {} original receipt payload/key changed",
            self.uuid
        );
        Ok(event)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReceiptEvidence {
    pub payload: Vec<u8>,
    pub receipt_key: (u64, u64),
    pub composed_at_ms: u64,
    pub handed_off_at_ms: Option<u64>,
    pub initial_response: Option<message_sender::SubmissionEvidence>,
    #[serde(default)]
    pub signed_submission: Option<message_sender::SignedSubmission>,
    #[serde(default)]
    pub submission_attempts: Vec<SubmissionAttempt>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubmissionAttempt {
    pub signed_submission: message_sender::SignedSubmission,
    pub response: Option<message_sender::SubmissionEvidence>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TxStatus {
    ComposeProof,
    SubmitMessage,
    PreparingSubmission,
    PreparedSubmission,
    NeedsReconciliation {
        diagnostic: String,
    },
    RetryableSubmission {
        diagnostic: String,
        retry_at_ms: u64,
    },
    Failed {
        diagnostic: String,
    },
    Completed,
}
impl_metered_service!(
    struct Metrics {
        total_transactions: IntCounter = IntCounter::new(
            "eth_gear_transaction_manager_total_transactions",
            "Total number of transactions processed by the transaction manager",
        ),
        completed_transactions: IntCounter = IntCounter::new(
            "eth_gear_transaction_manager_completed_transactions",
            "Total number of completed transactions",
        ),
        failed_transactions: IntCounter = IntCounter::new(
            "eth_geartransaction_manager_failed_transactions",
            "Total number of failed transactions",
        ),
    }
);

pub struct TransactionManager {
    /// Queue of transactions to be processed. Completed and failed
    /// transactions are moved to `completed` and `failed` maps.
    pub transactions: RwLock<BTreeMap<Uuid, Transaction>>,
    pub transactions_timestamp: RwLock<HashMap<Uuid, Instant>>,
    preparing_submissions: RwLock<HashSet<Uuid>>,

    pub completed: RwLock<BTreeMap<Uuid, Transaction>>,
    pub failed: RwLock<BTreeMap<Uuid, String>>,
    pub storage: Arc<dyn Storage>,

    metrics: Metrics,
}

impl MeteredService for TransactionManager {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl TransactionManager {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            transactions: RwLock::new(BTreeMap::new()),
            transactions_timestamp: RwLock::new(HashMap::with_capacity(CAPACITY)),
            preparing_submissions: RwLock::new(HashSet::new()),
            completed: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),
            storage,

            metrics: Metrics::new(),
        }
    }

    pub async fn fail_transaction(&self, tx_uuid: Uuid, reason: String) {
        self.failed.write().await.insert(tx_uuid, reason);
        self.metrics.failed_transactions.inc();
    }

    pub async fn add_transaction(&self, tx: Transaction) {
        self.metrics.total_transactions.inc();
        match tx.status {
            TxStatus::Completed => {
                self.completed.write().await.insert(tx.uuid, tx);
                self.metrics.completed_transactions.inc();
            }

            _ => {
                self.transactions.write().await.insert(tx.uuid, tx);
            }
        }
    }

    pub async fn run(
        self,
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
        mut proof_composer: ProofComposerIo,
        mut message_sender: MessageSenderIo,
    ) -> anyhow::Result<()> {
        loop {
            let result = self
                .process(
                    &mut message_paid_events,
                    &mut proof_composer,
                    &mut message_sender,
                )
                .await;

            match result {
                Ok(false) => {
                    log::error!("One of channels are closed, terminating transaction manager");
                    break;
                }
                Ok(true) => continue,
                Err(err) => {
                    log::error!("Transaction manager got error: {err:?}");
                    return Err(err);
                }
            }
        }

        Ok(())
    }

    pub async fn process(
        &self,
        message_paid_events: &mut UnboundedReceiver<TxHashWithSlot>,
        proof_composer: &mut ProofComposerIo,
        message_sender: &mut MessageSenderIo,
    ) -> anyhow::Result<bool> {
        // Reconcile pending work even when no channel receives a message.
        tokio::select! {
            _ = time::sleep(Duration::from_secs(12)) => {}
            response = message_sender.recv() => {
                let Some(response) = response else { return Ok(false) };
                let advance = matches!(
                    &response.status,
                    MessageStatus::Prepared | MessageStatus::Success { .. }
                );
                self.finalize_transaction(response).await?;
                if !advance {
                    return Ok(true);
                }
            }
            event = message_paid_events.recv() => {
                let Some(event) = event else { return Ok(false) };
                self.compose_proof(event).await?;
            }
            response = proof_composer.recv() => {
                let Some(proof_composer::Response { payload, tx_uuid }) = response else {
                    return Ok(false);
                };
                self.submit_message(tx_uuid, payload).await?;
            }

        }

        self.resume(message_sender, proof_composer).await
    }
    async fn resume(
        &self,
        message_sender: &mut MessageSenderIo,
        proof_composer: &mut ProofComposerIo,
    ) -> anyhow::Result<bool> {
        let transactions = self.transactions.read().await.clone();
        // Drain already-signed, not-yet-included calls by nonce before creating
        // any continuation. One signed owner prevents fresh nonce collisions.
        let pending_nonce = |transaction: &Transaction| {
            let signed = transaction.receipt.as_ref()?.signed_submission.as_ref()?;
            if signed.inclusion_block_number.is_none() {
                Some(signed.nonce)
            } else {
                signed
                    .native_reconciliation
                    .as_ref()
                    .filter(|continuation| continuation.inclusion_block_number.is_none())
                    .map(|continuation| continuation.nonce)
            }
        };
        let signed_owner = transactions
            .values()
            .filter(|transaction| {
                !matches!(
                    &transaction.status,
                    TxStatus::Failed { .. } | TxStatus::Completed
                ) && transaction
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.signed_submission.is_some())
            })
            .min_by_key(|transaction| {
                (
                    pending_nonce(transaction).is_none(),
                    pending_nonce(transaction).unwrap_or(u64::MAX),
                    transaction.uuid,
                )
            })
            .map(|transaction| transaction.uuid);
        let has_active_signed = signed_owner.is_some();
        for (_, tx) in transactions.iter() {
            if tx
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.signed_submission.is_some())
                && signed_owner != Some(tx.uuid)
            {
                continue;
            }
            let tx_uuid = tx.uuid;
            let retry_ready = matches!(
                &tx.status,
                TxStatus::RetryableSubmission { retry_at_ms, .. }
                    if *retry_at_ms <= message_sender::now_ms()
            );
            if !retry_ready {
                let timestamps = self.transactions_timestamp.read().await;
                if matches!(timestamps.get(&tx_uuid), Some(timestamp) if timestamp.elapsed() < Duration::from_secs(15 * 60))
                {
                    continue;
                }
            }

            match &tx.status {
                TxStatus::ComposeProof => {
                    if !proof_composer.compose_proof_for(tx_uuid, tx.tx.clone()) {
                        log::info!("Proof composer connection closed, exiting...");
                        return Ok(false);
                    }
                    log::info!("Transaction {tx_uuid} is enqueued for proof composition");
                }
                TxStatus::SubmitMessage | TxStatus::PreparingSubmission => {
                    if has_active_signed || !self.preparing_submissions.read().await.is_empty() {
                        continue;
                    }
                    let event = tx.receipt_event()?;
                    {
                        let mut active = self.transactions.write().await;
                        let Some(active_tx) = active.get_mut(&tx_uuid) else {
                            continue;
                        };
                        if !matches!(
                            &active_tx.status,
                            TxStatus::SubmitMessage | TxStatus::PreparingSubmission
                        ) {
                            continue;
                        }
                        active_tx.status = TxStatus::PreparingSubmission;
                    }
                    self.preparing_submissions.write().await.insert(tx_uuid);
                    self.storage.save(self).await?;
                    if !message_sender.prepare_message(tx_uuid, tx.tx.tx_hash, event) {
                        log::info!("Message sender connection closed, exiting...");
                        return Ok(false);
                    }
                    log::info!("Transaction {tx_uuid} is queued for signing only");
                }
                TxStatus::PreparedSubmission => {
                    let receipt = tx.receipt.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("Prepared submission has no receipt evidence")
                    })?;
                    let signed = receipt.signed_submission.clone().ok_or_else(|| {
                        anyhow::anyhow!("Prepared submission has no signed bytes")
                    })?;
                    let event = tx.receipt_event()?;
                    let diagnostic = format!(
                        "Exact signed submission {} is awaiting finalized reconciliation",
                        signed.extrinsic_hash
                    );
                    {
                        let mut active = self.transactions.write().await;
                        let Some(active_tx) = active.get_mut(&tx_uuid) else {
                            continue;
                        };
                        if !matches!(&active_tx.status, TxStatus::PreparedSubmission) {
                            continue;
                        }
                        active_tx.status = TxStatus::NeedsReconciliation {
                            diagnostic: diagnostic.clone(),
                        };
                        active_tx
                            .receipt
                            .as_mut()
                            .expect("prepared receipt was validated")
                            .handed_off_at_ms
                            .get_or_insert_with(message_sender::now_ms);
                    }
                    self.failed.write().await.insert(tx_uuid, diagnostic);
                    self.storage.save(self).await?;
                    if !message_sender.submit_prepared(tx_uuid, tx.tx.tx_hash, event, signed) {
                        log::info!("Message sender connection closed, exiting...");
                        return Ok(false);
                    }
                    log::info!("Persisted signed transaction {tx_uuid} is submitted");
                }
                TxStatus::NeedsReconciliation { .. } => {
                    let receipt = tx.receipt.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("Held transaction has no original receipt evidence")
                    })?;
                    if let Some(signed) = receipt.signed_submission.clone() {
                        let event = tx.receipt_event()?;
                        if !message_sender.submit_prepared(tx_uuid, tx.tx.tx_hash, event, signed) {
                            log::info!("Message sender connection closed, exiting...");
                            return Ok(false);
                        }
                        log::info!(
                            "Exact signed transaction for {tx_uuid} is enqueued for reconciliation"
                        );
                    } else if !message_sender.reconcile_receipt(tx_uuid, receipt.receipt_key) {
                        log::info!("Message sender connection closed, exiting...");
                        return Ok(false);
                    } else {
                        log::info!(
                            "Unsigned ambiguous receipt {:?} for {tx_uuid} is enqueued for status polling only",
                            receipt.receipt_key
                        );
                    }
                }
                TxStatus::RetryableSubmission { retry_at_ms, .. } => {
                    if *retry_at_ms > message_sender::now_ms() {
                        continue;
                    }
                    let mut active = self.transactions.write().await;
                    if let Some(active_tx) = active.get_mut(&tx_uuid) {
                        if matches!(&active_tx.status, TxStatus::RetryableSubmission { .. }) {
                            active_tx.status = TxStatus::SubmitMessage;
                        }
                    }
                    drop(active);
                    self.transactions_timestamp.write().await.remove(&tx_uuid);
                    self.storage.save(self).await?;
                    continue;
                }
                TxStatus::Failed { .. } | TxStatus::Completed => continue,
            }

            self.transactions_timestamp
                .write()
                .await
                .insert(tx_uuid, Instant::now());
        }

        Ok(true)
    }

    async fn compose_proof(&self, tx: TxHashWithSlot) -> anyhow::Result<()> {
        let tx_hash = tx.clone();
        let mut transactions = self.transactions.write().await;
        let already_seen = transactions.values().any(|existing| {
            existing.tx.slot_number == tx.slot_number && existing.tx.tx_hash == tx.tx_hash
        }) || self.completed.read().await.values().any(|existing| {
            existing.tx.slot_number == tx.slot_number && existing.tx.tx_hash == tx.tx_hash
        });

        if already_seen {
            log::info!("Ignoring replayed paid event {tx_hash:?}");
        } else {
            let transaction = Transaction::new(tx, TxStatus::ComposeProof);
            log::info!(
                "Received paid event {tx_hash:?}, transaction UUID: {}",
                transaction.uuid
            );
            transactions.insert(transaction.uuid, transaction);
        }
        drop(transactions);

        // The extractor can save blocks.json independently. Commit the transaction
        // first, or a later block save can persist this cleared cursor without it.
        self.storage.save(self).await?;
        self.storage
            .block_storage()
            .complete_transaction(&tx_hash)
            .await;
        self.storage.save_blocks().await
    }

    async fn submit_message(&self, tx_uuid: Uuid, payload: EthToVaraEvent) -> anyhow::Result<()> {
        let mut transactions = self.transactions.write().await;
        let Some(tx) = transactions.get_mut(&tx_uuid) else {
            log::warn!("Received proof for unknown transaction: {tx_uuid}");
            return Ok(());
        };
        if !matches!(tx.status, TxStatus::ComposeProof) {
            log::warn!(
                "Received proof for a transaction that is in {:?} state",
                tx.status
            );
            return Ok(());
        }
        let receipt_key = (payload.proof_block.block.slot, payload.transaction_index);
        tx.receipt = Some(ReceiptEvidence {
            payload: payload.encode(),
            receipt_key,
            composed_at_ms: message_sender::now_ms(),
            handed_off_at_ms: None,
            initial_response: None,
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        tx.receipt_event()?;
        tx.status = TxStatus::SubmitMessage;
        self.transactions_timestamp.write().await.remove(&tx_uuid);
        drop(transactions);
        self.storage.save(self).await
    }

    async fn finalize_transaction(&self, response: message_sender::Response) -> anyhow::Result<()> {
        let message_sender::Response {
            tx_uuid,
            mut status,
            submission,
            signed_submission,
        } = response;
        let mut transactions = self.transactions.write().await;
        let Some(tx) = transactions.get_mut(&tx_uuid) else {
            log::warn!("Received response for unknown transaction {tx_uuid}");
            return Ok(());
        };
        let receipt_key = tx
            .receipt
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Response arrived before its original receipt evidence for {tx_uuid}"
                )
            })?
            .receipt_key;
        match &status {
            MessageStatus::NeedsReconciliation {
                receipt_key: response_key,
                ..
            }
            | MessageStatus::RetryableNoop {
                receipt_key: response_key,
                ..
            } => {
                anyhow::ensure!(
                    receipt_key == *response_key,
                    "Response receipt identity changed for {tx_uuid}"
                );
            }
            _ => {}
        }
        if let Some(signed) = &signed_submission {
            anyhow::ensure!(
                signed.receipt_key == receipt_key,
                "Signed submission receipt identity changed for {tx_uuid}"
            );
            signed.validate_native_reconciliation()?;
            if let Some(previous) = tx
                .receipt
                .as_ref()
                .and_then(|receipt| receipt.signed_submission.as_ref())
            {
                anyhow::ensure!(
                    previous.same_request(signed),
                    "Original signed proof bytes/nonce/identity changed for {tx_uuid}"
                );
                anyhow::ensure!(
                    previous
                        .finalized_reply
                        .as_ref()
                        .is_none_or(|reply| signed.finalized_reply.as_ref() == Some(reply)),
                    "Original finalized proof reply identity changed for {tx_uuid}"
                );
                if let Some(original) = &previous.native_reconciliation {
                    anyhow::ensure!(
                        signed
                            .native_reconciliation
                            .as_ref()
                            .is_some_and(|current| original.same_request(current)),
                        "Original signed reconciliation bytes/nonce/identity changed for {tx_uuid}"
                    );
                    anyhow::ensure!(
                        original.finalized_reply.as_ref().is_none_or(|reply| signed
                            .native_reconciliation
                            .as_ref()
                            .and_then(|current| current.finalized_reply.as_ref())
                            == Some(reply)),
                        "Original finalized reconciliation reply identity changed for {tx_uuid}"
                    );
                }
            }
        }
        if let Some(evidence) = &submission {
            tx.receipt
                .as_mut()
                .expect("receipt was validated above")
                .initial_response
                .get_or_insert_with(|| evidence.clone());
        }
        if let Some(signed) = signed_submission.clone() {
            tx.receipt
                .as_mut()
                .expect("receipt was validated above")
                .signed_submission = Some(signed);
        }
        if let MessageStatus::Success {
            receipt_observation,
        } = &status
        {
            let original = tx
                .receipt
                .as_ref()
                .and_then(|receipt| receipt.signed_submission.as_ref());
            if receipt_observation.as_ref().is_none_or(|observation| observation.finalized_block_number.is_none())
                || original.is_none_or(|signed| !signed.has_finalized_dispatch_reply()
                    || signed.native_reconciliation.as_ref().is_some_and(|continuation|
                        !continuation.has_finalized_dispatch_reply()
                            || continuation.finalized_reply.as_ref().is_none_or(|reply|
                                !matches!(message_sender::decode_complete_reply::<vft_manager_client::vft_manager::io::ReconcileReceipt>(&reply.payload),
                                    Ok(Ok(vft_manager_client::ReceiptStatus::Processed))))))
                || submission.as_ref().is_none_or(|evidence| evidence.manager_reply.is_none()) {
                status = MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: "Completion omitted the original finalized signed dispatch/reply evidence".into(),
                    receipt_observation: receipt_observation.clone(),
                };
            }
        }
        let mut tx = transactions
            .remove(&tx_uuid)
            .expect("transaction was validated above");
        self.preparing_submissions.write().await.remove(&tx_uuid);
        log::info!("Received response for transaction {tx_uuid}: {status:?}");
        match status {
            MessageStatus::Prepared => {
                anyhow::ensure!(
                    tx.receipt
                        .as_ref()
                        .and_then(|receipt| receipt.signed_submission.as_ref())
                        .is_some(),
                    "Prepared response for {tx_uuid} omitted signed transaction bytes"
                );
                tx.status = TxStatus::PreparedSubmission;
                transactions.insert(tx_uuid, tx);
                self.transactions_timestamp.write().await.remove(&tx_uuid);
            }
            MessageStatus::PrepareFailed(diagnostic) => {
                tx.status = TxStatus::PreparingSubmission;
                self.fail_transaction(tx_uuid, diagnostic).await;
                transactions.insert(tx_uuid, tx);
                self.transactions_timestamp
                    .write()
                    .await
                    .insert(tx_uuid, Instant::now());
            }
            MessageStatus::Success {
                receipt_observation,
            } => {
                if let Some(observation) = receipt_observation {
                    tx.receipt_observation = Some(observation);
                }
                tx.status = TxStatus::Completed;
                self.failed.write().await.remove(&tx.uuid);
                self.completed.write().await.insert(tx.uuid, tx);
                self.metrics.completed_transactions.inc();
                self.transactions_timestamp.write().await.remove(&tx_uuid);
            }
            MessageStatus::Failure(message) => {
                if let Some(signed) = tx
                    .receipt
                    .as_mut()
                    .expect("receipt was validated above")
                    .signed_submission
                    .take()
                {
                    tx.receipt
                        .as_mut()
                        .expect("receipt was validated above")
                        .submission_attempts
                        .push(SubmissionAttempt {
                            signed_submission: signed,
                            response: submission,
                        });
                }
                tx.status = TxStatus::Failed {
                    diagnostic: message.clone(),
                };
                self.fail_transaction(tx_uuid, message).await;
                transactions.insert(tx_uuid, tx);
            }
            MessageStatus::RetryableNoop {
                diagnostic,
                receipt_observation,
                ..
            } => {
                let receipt = tx.receipt.as_mut().expect("receipt was validated above");
                let signed = receipt.signed_submission.take().ok_or_else(|| {
                    anyhow::anyhow!("Retryable outcome for {tx_uuid} has no signed attempt")
                })?;
                receipt.submission_attempts.push(SubmissionAttempt {
                    signed_submission: signed,
                    response: submission,
                });
                if let Some(observation) = receipt_observation {
                    tx.receipt_observation = Some(observation);
                }
                tx.status = TxStatus::RetryableSubmission {
                    diagnostic: diagnostic.clone(),
                    retry_at_ms: message_sender::now_ms().saturating_add(60_000),
                };
                self.fail_transaction(tx_uuid, diagnostic).await;
                transactions.insert(tx_uuid, tx);
                self.transactions_timestamp
                    .write()
                    .await
                    .insert(tx_uuid, Instant::now());
            }
            MessageStatus::NeedsReconciliation {
                diagnostic,
                receipt_observation,
                ..
            } => {
                if let Some(observation) = receipt_observation {
                    tx.receipt_observation = Some(observation);
                }
                tx.status = TxStatus::NeedsReconciliation {
                    diagnostic: diagnostic.clone(),
                };
                self.fail_transaction(tx_uuid, diagnostic).await;
                transactions.insert(tx_uuid, tx);
                self.transactions_timestamp.write().await.remove(&tx_uuid);
            }
        }
        drop(transactions);
        self.storage.save(self).await
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::message_relayer::common::EthereumSlotNumber;
    use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel};

    fn runtime_identity() -> super::super::storage::InboundRuntimeIdentity {
        use primitive_types::{H160, H256};
        super::super::storage::InboundRuntimeIdentity {
            ethereum_chain_id: 560048,
            ethereum_genesis_hash: H256::repeat_byte(1),
            ethereum_start_block: 70,
            erc20_manager_address: Some(H160::repeat_byte(2)),
            bridging_payment_address: None,
            gear_genesis_hash: H256::repeat_byte(3),
            vft_manager_address: H256::repeat_byte(4),
            checkpoint_light_client_address: H256::repeat_byte(5),
            historical_proxy_address: H256::repeat_byte(6),
            gear_sender: [7; 32],
        }
    }

    fn signed_submission(receipt_key: (u64, u64)) -> message_sender::SignedSubmission {
        message_sender::SignedSubmission {
            finalized_dispatch_error: None,
            finalized_reply: None,
            native_reconciliation: None,
            chain_genesis_hash: "0x01".into(),
            manager: "0x02".into(),
            historical_proxy: "0x03".into(),
            sender: "source-signer".into(),
            receipt_key,
            payload_hash: "0x04".into(),
            nonce: 12,
            extrinsic_hash: "0x05".into(),
            raw_extrinsic: vec![1, 2, 3, 4],
            prepared_finalized_number: 50,
            prepared_finalized_hash: "0x06".into(),
            scanned_finalized_number: 50,
            scanned_finalized_hash: "0x06".into(),
            reply_scanned_finalized_number: 50,
            reply_scanned_finalized_hash: "0x06".into(),
            inclusion_block_number: None,
            inclusion_block_hash: None,
            message_id: None,
        }
    }

    #[tokio::test]
    async fn native_reconciliation_journals_separate_nonce_before_broadcast_and_restarts_exactly() {
        use super::super::storage::JSONStorage;
        use sails_rs::calls::ActionIo;
        use subxt::config::{substrate::BlakeTwo256, Hasher};
        use vft_manager_client::vft_manager::io::ReconcileReceipt;
        let path =
            std::env::temp_dir().join(format!("inbound-native-continuation-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let (lock, event) = receipt_fixtures().remove(0);
        let key = (lock.slot_number.0, event.transaction_index);
        let mut original = signed_submission(key);
        original.inclusion_block_number = Some(51);
        original.inclusion_block_hash = Some("0x07".into());
        original.message_id = Some("0x08".into());
        original.finalized_reply = Some(message_sender::FinalizedReplyEvidence {
            payload: vec![0x01],
            observation: message_sender::FinalizedReceiptObservation {
                finalized_block_number: Some(52),
                finalized_block_hash: "0x09".into(),
            },
        });
        let mut tx = Transaction::new(
            lock,
            TxStatus::NeedsReconciliation {
                diagnostic: "original native pending".into(),
            },
        );
        tx.receipt = Some(ReceiptEvidence {
            payload: event.encode(),
            receipt_key: key,
            composed_at_ms: 10,
            handed_off_at_ms: Some(20),
            initial_response: None,
            signed_submission: Some(original.clone()),
            submission_attempts: Vec::new(),
        });
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        storage.save(&manager).await.unwrap();
        // Schema5 history gains optional fields without losing raw bytes or cursors.
        let committed = tokio::fs::read(path.join("state.json")).await.unwrap();
        let mut legacy: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        legacy["schema_version"] = serde_json::json!(5);
        legacy["transactions"][uuid.to_string()]["receipt"]["signed_submission"]
            .as_object_mut()
            .unwrap()
            .remove("finalized_reply");
        legacy["operator_extension"] = serde_json::json!({ "owner": "original" });
        tokio::fs::write(
            path.join("state.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .await
        .unwrap();
        let mut continuation = signed_submission(key);
        continuation.historical_proxy = original.historical_proxy.clone();
        continuation.prepared_finalized_number = 52;
        continuation.prepared_finalized_hash = "0x09".into();
        continuation.scanned_finalized_number = 52;
        continuation.scanned_finalized_hash = "0x09".into();
        continuation.reply_scanned_finalized_number = 52;
        continuation.reply_scanned_finalized_hash = "0x09".into();
        continuation.nonce = original.nonce + 1;
        continuation.payload_hash = format!(
            "{:#x}",
            alloy_primitives::keccak256(ReconcileReceipt::encode_call(key.0, key.1))
        );
        continuation.raw_extrinsic = vec![5, 6, 7, 8];
        continuation.extrinsic_hash =
            format!("{:#x}", BlakeTwo256.hash(&continuation.raw_extrinsic));
        let mut prepared = original.clone();
        prepared.native_reconciliation = Some(Box::new(continuation.clone()));
        for mutation in ["nonce", "target", "raw", "nested"] {
            let mut invalid = prepared.clone();
            let child = invalid.native_reconciliation.as_mut().unwrap();
            match mutation {
                "nonce" => child.nonce = original.nonce,
                "target" => child.historical_proxy = original.manager.clone(),
                "raw" => child.raw_extrinsic[0] ^= 1,
                "nested" => child.native_reconciliation = Some(Box::new(continuation.clone())),
                _ => unreachable!(),
            }
            assert!(
                invalid.validate_native_reconciliation().is_err(),
                "{mutation}"
            );
        }
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::Prepared,
                submission: None,
                signed_submission: Some(prepared.clone()),
            })
            .await
            .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(path.join("state.json")).await.unwrap())
                .unwrap();
        assert_eq!(persisted["schema_version"], 6);
        assert_eq!(
            persisted["operator_extension"],
            legacy["operator_extension"]
        );
        let signed = &persisted["transactions"][uuid.to_string()]["receipt"]["signed_submission"];
        assert_eq!(
            signed["raw_extrinsic"],
            serde_json::to_value(&original.raw_extrinsic).unwrap()
        );
        assert_eq!(
            signed["native_reconciliation"]["raw_extrinsic"],
            serde_json::to_value(&continuation.raw_extrinsic).unwrap()
        );
        drop(manager);
        for _ in 0..2 {
            let restored = TransactionManager::new(storage.clone());
            storage.load(&restored).await.unwrap();
            let (requests, mut sent) = unbounded_channel();
            let (_responses, received) = unbounded_channel();
            let mut sender = MessageSenderIo::new(requests, received);
            let (requests, mut proof_requests) = unbounded_channel();
            let (_responses, received) = unbounded_channel();
            let mut composer = ProofComposerIo::new(requests, received);
            restored.resume(&mut sender, &mut composer).await.unwrap();
            match sent.try_recv().unwrap() {
                message_sender::Request::SubmitPrepared {
                    tx_uuid,
                    prepared: original,
                    ..
                } => {
                    assert_eq!(tx_uuid, uuid);
                    assert_eq!(original.1, prepared);
                    assert_eq!(original.0.encode(), event.encode());
                }
                other => panic!("continuation restart replaced the original proof: {other:?}"),
            }
            assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
            assert!(matches!(
                proof_requests.try_recv(),
                Err(TryRecvError::Empty)
            ));
            assert_eq!(
                restored.transactions.read().await[&uuid]
                    .receipt
                    .as_ref()
                    .unwrap()
                    .handed_off_at_ms,
                Some(20)
            );
        }
        let restored = TransactionManager::new(storage.clone());
        storage.load(&restored).await.unwrap();
        let observation = message_sender::FinalizedReceiptObservation {
            finalized_block_number: Some(54),
            finalized_block_hash: "0x0c".into(),
        };
        let evidence = message_sender::SubmissionEvidence {
            observed_at_ms: 30,
            manager: primitive_types::H256::repeat_byte(4),
            historical_proxy: primitive_types::H256::repeat_byte(6),
            sender: "source-signer".into(),
            manager_reply: Some(vec![0]),
            error: None,
        };
        restored
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::Success {
                    receipt_observation: Some(observation.clone()),
                },
                submission: Some(evidence.clone()),
                signed_submission: Some(prepared.clone()),
            })
            .await
            .unwrap();
        assert!(restored.completed.read().await.is_empty(), "wrapper Delivered alone cannot drain the journal before the original continuation reply");
        let mut completed = prepared.clone();
        let child = completed.native_reconciliation.as_mut().unwrap();
        child.inclusion_block_number = Some(53);
        child.inclusion_block_hash = Some("0x0a".into());
        child.message_id = Some("0x0b".into());
        let result: <ReconcileReceipt as ActionIo>::Reply =
            Ok(vft_manager_client::ReceiptStatus::Processed);
        let mut reply = ReconcileReceipt::ROUTE.to_vec();
        reply.extend(result.encode());
        child.finalized_reply = Some(message_sender::FinalizedReplyEvidence {
            payload: reply,
            observation: observation.clone(),
        });
        restored
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::Success {
                    receipt_observation: Some(observation),
                },
                submission: Some(evidence),
                signed_submission: Some(completed.clone()),
            })
            .await
            .unwrap();
        assert_eq!(
            restored.completed.read().await[&uuid]
                .receipt
                .as_ref()
                .unwrap()
                .signed_submission
                .as_ref(),
            Some(&completed)
        );
        assert_eq!(completed.raw_extrinsic, original.raw_extrinsic);
        assert_eq!(completed.nonce, original.nonce);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn prepared_submission_is_journaled_before_broadcast_state() {
        let manager = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        let receipt_key = (12, 8);
        let signed = signed_submission(receipt_key);
        let mut tx = Transaction::new(
            TxHashWithSlot {
                slot_number: EthereumSlotNumber(7),
                tx_hash: Default::default(),
            },
            TxStatus::PreparingSubmission,
        );
        tx.receipt = Some(ReceiptEvidence {
            payload: vec![7, 8, 9],
            receipt_key,
            composed_at_ms: 10,
            handed_off_at_ms: None,
            initial_response: None,
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::Prepared,
                submission: None,
                signed_submission: Some(signed.clone()),
            })
            .await
            .unwrap();

        let transactions = manager.transactions.read().await;
        let stored = &transactions[&uuid];
        assert!(matches!(stored.status, TxStatus::PreparedSubmission));
        let receipt = stored.receipt.as_ref().unwrap();
        assert_eq!(receipt.signed_submission.as_ref(), Some(&signed));
        assert_eq!(receipt.signed_submission.as_ref().unwrap().nonce, 12);
        assert_eq!(
            receipt.signed_submission.as_ref().unwrap().raw_extrinsic,
            vec![1, 2, 3, 4]
        );
        assert!(manager.completed.read().await.is_empty());
    }

    #[tokio::test]
    async fn finalized_retryable_noop_moves_exact_attempt_to_history() {
        let manager = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        let receipt_key = (12, 8);
        let signed = signed_submission(receipt_key);
        let evidence = message_sender::SubmissionEvidence {
            observed_at_ms: 70,
            manager: primitive_types::H256::repeat_byte(2),
            historical_proxy: primitive_types::H256::repeat_byte(3),
            sender: "source-signer".into(),
            manager_reply: Some(vec![0xaa, 0xbb]),
            error: Some("finalized ReplyFailure".into()),
        };
        let mut tx = Transaction::new(
            TxHashWithSlot {
                slot_number: EthereumSlotNumber(7),
                tx_hash: Default::default(),
            },
            TxStatus::NeedsReconciliation {
                diagnostic: "in flight".into(),
            },
        );
        tx.receipt = Some(ReceiptEvidence {
            payload: vec![7, 8, 9],
            receipt_key,
            composed_at_ms: 10,
            handed_off_at_ms: Some(20),
            initial_response: None,
            signed_submission: Some(signed.clone()),
            submission_attempts: Vec::new(),
        });
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::RetryableNoop {
                    receipt_key,
                    diagnostic: "finalized per-log failure".into(),
                    receipt_observation: None,
                },
                submission: Some(evidence.clone()),
                signed_submission: Some(signed.clone()),
            })
            .await
            .unwrap();

        let transactions = manager.transactions.read().await;
        let stored = &transactions[&uuid];
        assert!(matches!(
            stored.status,
            TxStatus::RetryableSubmission { .. }
        ));
        let receipt = stored.receipt.as_ref().unwrap();
        assert!(receipt.signed_submission.is_none());
        assert_eq!(receipt.submission_attempts.len(), 1);
        assert_eq!(receipt.submission_attempts[0].signed_submission, signed);
        assert_eq!(
            receipt.submission_attempts[0].response.as_ref(),
            Some(&evidence)
        );
    }

    #[tokio::test]
    async fn held_unsigned_processed_receipt_never_completes_or_resubmits() {
        let manager = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        let (lock, event) = receipt_fixtures().remove(0);
        let receipt_key = (lock.slot_number.0, event.transaction_index);
        let payload = event.encode();
        let failure = message_sender::SubmissionEvidence {
            observed_at_ms: 30,
            manager: primitive_types::H256::repeat_byte(1),
            historical_proxy: primitive_types::H256::repeat_byte(2),
            sender: "source-signer".into(),
            manager_reply: Some(vec![9, 8, 7]),
            error: Some("Internal: sharded map error: capacity overflow".into()),
        };
        let mut tx = Transaction::new(
            lock,
            TxStatus::NeedsReconciliation {
                diagnostic: "submission outcome pending".into(),
            },
        );
        tx.receipt = Some(ReceiptEvidence {
            payload: payload.clone(),
            receipt_key,
            composed_at_ms: 10,
            handed_off_at_ms: Some(20),
            initial_response: None,
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: failure.error.clone().unwrap(),
                    receipt_observation: None,
                },
                submission: Some(failure.clone()),
                signed_submission: None,
            })
            .await
            .unwrap();
        let unknown = message_sender::FinalizedReceiptObservation {
            finalized_block_number: Some(73),
            finalized_block_hash: "0xabcd".into(),
        };
        let (send_requests, mut sent) = unbounded_channel();
        let (send_responses, received) = unbounded_channel();
        let mut sender = MessageSenderIo::new(send_requests, received);
        let (proof_requests, mut composed) = unbounded_channel();
        let (_proof_responses, received) = unbounded_channel();
        let mut composer = ProofComposerIo::new(proof_requests, received);
        let (_paid, mut paid) = unbounded_channel();
        for (diagnostic, observation) in [
            ("Unknown at finalized block 73", Some(unknown.clone())),
            ("Could not pin the next finalized receipt query", None),
        ] {
            send_responses
                .send(message_sender::Response {
                    tx_uuid: uuid,
                    status: MessageStatus::NeedsReconciliation {
                        receipt_key,
                        diagnostic: diagnostic.into(),
                        receipt_observation: observation,
                    },
                    submission: None,
                    signed_submission: None,
                })
                .unwrap();
            assert!(tokio::time::timeout(
                Duration::from_secs(5),
                manager.process(&mut paid, &mut composer, &mut sender),
            )
            .await
            .expect("an unresolved response must be recorded without a new polling request")
            .unwrap());
            assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
            assert!(matches!(composed.try_recv(), Err(TryRecvError::Empty)));
            let transactions = manager.transactions.read().await;
            let original = transactions[&uuid].receipt.as_ref().unwrap();
            assert_eq!(original.payload, payload);
            assert_eq!(original.receipt_key, receipt_key);
            assert_eq!(original.initial_response.as_ref(), Some(&failure));
        }
        assert_eq!(
            manager.transactions.read().await[&uuid]
                .receipt_observation
                .as_ref(),
            Some(&unknown)
        );
        assert!(manager.resume(&mut sender, &mut composer).await.unwrap());
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::ReceiptStatus {
            tx_uuid: request_uuid, receipt_key: request_key,
        }) if request_uuid == uuid && request_key == receipt_key)
        );
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        assert!(matches!(composed.try_recv(), Err(TryRecvError::Empty)));
        let processed = message_sender::FinalizedReceiptObservation {
            finalized_block_number: Some(74),
            finalized_block_hash: "0xbeef".into(),
        };
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::Success {
                    receipt_observation: Some(processed.clone()),
                },
                submission: None,
                signed_submission: None,
            })
            .await
            .unwrap();
        assert!(manager.failed.read().await.contains_key(&uuid));
        assert!(manager.completed.read().await.is_empty());
        let path =
            std::env::temp_dir().join(format!("gear-eth-to-gear-processed-{}", Uuid::now_v7()));
        let storage = super::super::storage::JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        storage.save(&manager).await.unwrap();
        let restored = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        storage.load(&restored).await.unwrap();
        let held = restored.transactions.read().await;
        assert!(matches!(
            held[&uuid].status,
            TxStatus::NeedsReconciliation { .. }
        ));
        assert_eq!(held[&uuid].receipt_observation.as_ref(), Some(&processed));
        let original = held[&uuid].receipt.as_ref().unwrap();
        assert_eq!(original.payload, payload);
        assert_eq!(original.receipt_key, receipt_key);
        assert_eq!(original.initial_response.as_ref(), Some(&failure));
        assert_eq!(original.handed_off_at_ms, Some(20));
        drop(held);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn replayed_paid_events_do_not_allocate_new_transactions() {
        let manager = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        let identity = TxHashWithSlot {
            slot_number: EthereumSlotNumber(7),
            tx_hash: Default::default(),
        };
        let mut active = Transaction::new(
            identity.clone(),
            TxStatus::NeedsReconciliation {
                diagnostic: "held".into(),
            },
        );
        active.receipt = Some(ReceiptEvidence {
            payload: vec![1],
            receipt_key: (9, 4),
            composed_at_ms: 10,
            handed_off_at_ms: Some(20),
            initial_response: None,
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        let original = active.receipt.clone();
        let active_uuid = active.uuid;
        manager
            .transactions
            .write()
            .await
            .insert(active_uuid, active);

        manager.compose_proof(identity.clone()).await.unwrap();
        assert_eq!(manager.transactions.read().await.len(), 1);
        assert!(manager.transactions.read().await.contains_key(&active_uuid));

        manager.transactions.write().await.remove(&active_uuid);
        let mut completed = Transaction::new(identity.clone(), TxStatus::Completed);
        completed.receipt = original;
        let completed_uuid = completed.uuid;
        manager
            .completed
            .write()
            .await
            .insert(completed_uuid, completed);
        manager.compose_proof(identity.clone()).await.unwrap();

        assert!(manager.transactions.read().await.is_empty());
        assert_eq!(manager.completed.read().await.len(), 1);
        assert!(manager.completed.read().await.contains_key(&completed_uuid));
        let path = std::env::temp_dir().join(format!("gear-eth-to-gear-replay-{}", Uuid::now_v7()));
        let storage = super::super::storage::JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        storage.save(&manager).await.unwrap();

        let restored = TransactionManager::new(Arc::new(super::super::storage::NoStorage::new()));
        storage.load(&restored).await.unwrap();
        restored.compose_proof(identity).await.unwrap();
        assert_eq!(restored.transactions.read().await.len(), 1);
        assert!(matches!(
            restored.transactions.read().await[&completed_uuid].status,
            TxStatus::NeedsReconciliation { .. }
        ));
        assert!(restored.completed.read().await.is_empty());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn extractor_block_save_after_ack_cannot_lose_the_deposit_on_restart() {
        use super::super::storage::JSONStorage;
        use crate::message_relayer::common::EthereumBlockNumber;
        use ethereum_client::TxHash;

        let path = std::env::temp_dir().join(format!("gear-inbound-ack-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let deposit = TxHashWithSlot {
            slot_number: EthereumSlotNumber(7),
            tx_hash: TxHash::from([7; 32]),
        };
        storage
            .block_storage()
            .add_block(
                deposit.slot_number,
                EthereumBlockNumber(70),
                primitive_types::H256::repeat_byte(9),
                [deposit.tx_hash].into_iter(),
            )
            .await
            .unwrap();
        storage.save(&manager).await.unwrap();

        manager.compose_proof(deposit.clone()).await.unwrap();
        let durable = Arc::new(JSONStorage::new(&path));
        durable
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let durable_manager = TransactionManager::new(durable.clone());
        durable.load(&durable_manager).await.unwrap();
        assert!(
            !durable
                .block_storage()
                .is_transaction_pending(deposit.slot_number, deposit.tx_hash)
                .await
        );
        assert!(durable_manager
            .transactions
            .read()
            .await
            .values()
            .any(|tx| tx.tx.tx_hash == deposit.tx_hash));
        // The independent extractor then commits a later block before the process dies.
        storage
            .block_storage()
            .add_block(
                EthereumSlotNumber(8),
                EthereumBlockNumber(71),
                primitive_types::H256::repeat_byte(9),
                [TxHash::from([8; 32])].into_iter(),
            )
            .await
            .unwrap();
        storage.save_blocks().await.unwrap();
        drop(manager);
        drop(storage);

        let restored_storage = Arc::new(JSONStorage::new(&path));
        restored_storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let restored = TransactionManager::new(restored_storage.clone());
        restored_storage.load(&restored).await.unwrap();
        assert!(restored.transactions.read().await.values().any(|tx| {
            tx.tx.tx_hash == deposit.tx_hash
                && tx.tx.slot_number == deposit.slot_number
                && matches!(tx.status, TxStatus::ComposeProof)
        }));
        assert!(
            !restored_storage
                .block_storage()
                .is_transaction_pending(deposit.slot_number, deposit.tx_hash)
                .await
        );
        assert_eq!(
            restored_storage
                .block_storage()
                .unprocessed_blocks()
                .await
                .unprocessed,
            vec![EthereumBlockNumber(71)]
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    pub(in crate::message_relayer::eth_to_gear) fn receipt_fixtures(
    ) -> Vec<(TxHashWithSlot, EthToVaraEvent)> {
        use alloy_rlp::Encodable;
        use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof};
        use ethereum_common::{
            beacon::electra::Block,
            utils::{self, BeaconBlockHeaderResponse, BeaconBlockResponse},
        };
        use std::io::Read;
        #[derive(Deserialize)]
        struct Receipts {
            result: Vec<alloy::rpc::types::TransactionReceipt>,
        }
        #[derive(Deserialize)]
        struct Fixture {
            tx_hash: ethereum_client::TxHash,
            tx_index: u64,
            slot_number: u64,
            receipts: Receipts,
            block: BeaconBlockResponse<Block>,
            headers: Vec<BeaconBlockHeaderResponse>,
        }
        let bytes: &[u8] = include_bytes!("../../../../tests/src/relayer/transactions.json.zst");
        let mut decoder = ruzstd::StreamingDecoder::new(bytes).unwrap();
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        let mut fixtures: Vec<Fixture> = serde_json::from_slice(&decoded).unwrap();
        fixtures.sort_by_key(|fixture| fixture.tx_hash);
        fixtures
            .into_iter()
            .take(2)
            .map(|fixture| {
                let receipts = fixture
                    .receipts
                    .result
                    .iter()
                    .map(|receipt| {
                        (
                            receipt.transaction_index.unwrap(),
                            utils::map_receipt_envelope(receipt.as_ref()),
                        )
                    })
                    .collect::<Vec<_>>();
                let merkle = utils::generate_merkle_proof(fixture.tx_index, &receipts).unwrap();
                let mut receipt_rlp = Vec::new();
                Encodable::encode(&merkle.receipt, &mut receipt_rlp);
                let block = fixture.block.data.message;
                let event = EthToVaraEvent {
                    proof_block: BlockInclusionProof {
                        block: BlockGenericForBlockBody {
                            slot: block.slot,
                            proposer_index: block.proposer_index,
                            parent_root: block.parent_root,
                            state_root: block.state_root,
                            body: block.body.into(),
                        },
                        headers: fixture
                            .headers
                            .into_iter()
                            .map(|header| header.data.header.message)
                            .collect(),
                    },
                    proof: merkle.proof,
                    transaction_index: fixture.tx_index,
                    receipt_rlp,
                };
                (
                    TxHashWithSlot {
                        slot_number: EthereumSlotNumber(fixture.slot_number),
                        tx_hash: fixture.tx_hash,
                    },
                    event,
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn inherited_schema_four_proof_holds_without_rewriting_original_identity() {
        use super::super::storage::JSONStorage;
        let path = std::env::temp_dir().join(format!("inbound-unbound-proof-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let (lock, event) = receipt_fixtures().remove(0);
        let transaction = Transaction::new(lock, TxStatus::ComposeProof);
        let uuid = transaction.uuid;
        manager.add_transaction(transaction).await;
        manager.submit_message(uuid, event).await.unwrap();
        let state_path = path.join("state.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&state_path).await.unwrap()).unwrap();
        legacy["schema_version"] = serde_json::json!(4);
        let original = serde_json::to_vec(&legacy).unwrap();
        tokio::fs::write(&state_path, &original).await.unwrap();
        drop(manager);
        drop(storage);

        let restored = JSONStorage::new(&path);
        let accepted = restored
            .bind_runtime_identity(runtime_identity())
            .await
            .is_ok();
        let retained = tokio::fs::read(&state_path).await.unwrap();
        tokio::fs::remove_dir_all(path).await.unwrap();
        assert!(
            !accepted,
            "inherited proof lacks the original-transaction authentication guarantee"
        );
        assert_eq!(
            retained, original,
            "HOLD must preserve the original UUID, hash and proof bytes"
        );
    }

    #[tokio::test]
    async fn two_queued_locks_restart_without_resigning_the_active_submission() {
        use super::super::storage::JSONStorage;
        let path = std::env::temp_dir().join(format!("inbound-two-locks-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let fixtures = receipt_fixtures();
        assert_eq!(fixtures.len(), 2);
        let mut uuids = Vec::new();
        for (lock, event) in &fixtures {
            let transaction = Transaction::new(lock.clone(), TxStatus::ComposeProof);
            let uuid = transaction.uuid;
            manager.add_transaction(transaction).await;
            manager.submit_message(uuid, event.clone()).await.unwrap();
            uuids.push(uuid);
        }
        let (requests, mut sent) = unbounded_channel();
        let (responses, received) = unbounded_channel();
        let mut sender = MessageSenderIo::new(requests, received);
        let (requests, _proof_requests) = unbounded_channel();
        let (_responses, received) = unbounded_channel();
        let mut composer = ProofComposerIo::new(requests, received);
        let first_signed =
            signed_submission((fixtures[0].0.slot_number.0, fixtures[0].1.transaction_index));
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuids[0],
                status: MessageStatus::Prepared,
                submission: None,
                signed_submission: Some(first_signed.clone()),
            })
            .await
            .unwrap();
        drop(manager);
        let manager = TransactionManager::new(storage.clone());
        storage.load(&manager).await.unwrap();
        manager.resume(&mut sender, &mut composer).await.unwrap();
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::SubmitPrepared {
            tx_uuid, prepared, ..
        }) if tx_uuid == uuids[0] && prepared.1 == first_signed)
        );
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        drop(manager);
        let restored = TransactionManager::new(storage.clone());
        storage.load(&restored).await.unwrap();
        restored.resume(&mut sender, &mut composer).await.unwrap();
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::SubmitPrepared {
            tx_uuid, prepared, ..
        }) if tx_uuid == uuids[0] && prepared.1 == first_signed)
        );
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        let (_paid, mut paid) = unbounded_channel();
        tokio::time::timeout(Duration::from_secs(5), async {
            let (result, ()) = tokio::join!(
                restored.process(&mut paid, &mut composer, &mut sender),
                async {
                    tokio::task::yield_now().await;
                    let mut delivered = first_signed.clone();
                    delivered.inclusion_block_number = Some(51);
                    delivered.inclusion_block_hash = Some("0x07".into());
                    delivered.message_id = Some("0x08".into());
                    delivered.finalized_reply = Some(message_sender::FinalizedReplyEvidence {
                        payload: vec![0],
                        observation: message_sender::FinalizedReceiptObservation {
                            finalized_block_number: Some(52),
                            finalized_block_hash: "0x09".into(),
                        },
                    });
                    responses
                        .send(message_sender::Response {
                            tx_uuid: uuids[0],
                            status: MessageStatus::Success {
                                receipt_observation: Some(
                                    message_sender::FinalizedReceiptObservation {
                                        finalized_block_number: Some(52),
                                        finalized_block_hash: "0x09".into(),
                                    },
                                ),
                            },
                            submission: Some(message_sender::SubmissionEvidence {
                                observed_at_ms: 30,
                                manager: primitive_types::H256::repeat_byte(4),
                                historical_proxy: primitive_types::H256::repeat_byte(6),
                                sender: "source-signer".into(),
                                manager_reply: Some(vec![0]),
                                error: None,
                            }),
                            signed_submission: Some(delivered),
                        })
                        .unwrap();
                }
            );
            assert!(result.unwrap());
        })
        .await
        .expect("sender completion must wake the idle transaction manager");
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::PrepareMessage {
            tx_uuid, tx_hash, payload,
        }) if tx_uuid == uuids[1] && tx_hash == fixtures[1].0.tx_hash && payload.encode() == fixtures[1].1.encode())
        );
        restored.resume(&mut sender, &mut composer).await.unwrap();
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        let mut second_signed =
            signed_submission((fixtures[1].0.slot_number.0, fixtures[1].1.transaction_index));
        second_signed.nonce += 1;
        responses
            .send(message_sender::Response {
                tx_uuid: uuids[1],
                status: MessageStatus::Prepared,
                submission: None,
                signed_submission: Some(second_signed.clone()),
            })
            .unwrap();
        assert!(tokio::time::timeout(
            Duration::from_secs(5),
            restored.process(&mut paid, &mut composer, &mut sender),
        )
        .await
        .expect("a queued prepared signature must not wait for the idle timer")
        .unwrap());
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::SubmitPrepared {
            tx_uuid, prepared, ..
        }) if tx_uuid == uuids[1] && prepared.1 == second_signed)
        );
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        drop(restored);
        let restored = TransactionManager::new(storage.clone());
        storage.load(&restored).await.unwrap();
        restored.resume(&mut sender, &mut composer).await.unwrap();
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::SubmitPrepared {
            tx_uuid, prepared, ..
        }) if tx_uuid == uuids[1] && prepared.1 == second_signed)
        );
        assert!(
            matches!(sent.try_recv(), Err(TryRecvError::Empty)),
            "restored signed continuations must share one nonce owner"
        );
        assert_eq!(
            restored.transactions.read().await[&uuids[1]]
                .receipt
                .as_ref()
                .unwrap()
                .payload,
            fixtures[1].1.encode()
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn retry_ready_restart_retains_original_attempt_and_resumes_unsigned_proof() {
        use super::super::storage::JSONStorage;
        let path = std::env::temp_dir().join(format!("inbound-retry-ready-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let (lock, event) = receipt_fixtures().remove(0);
        let receipt_key = (lock.slot_number.0, event.transaction_index);
        let transaction = Transaction::new(lock, TxStatus::ComposeProof);
        let uuid = transaction.uuid;
        manager.add_transaction(transaction).await;
        manager.submit_message(uuid, event.clone()).await.unwrap();
        let signed = signed_submission(receipt_key);
        {
            let mut transactions = manager.transactions.write().await;
            let transaction = transactions.get_mut(&uuid).unwrap();
            transaction.status = TxStatus::NeedsReconciliation {
                diagnostic: "original handoff".into(),
            };
            let receipt = transaction.receipt.as_mut().unwrap();
            receipt.signed_submission = Some(signed.clone());
            receipt.handed_off_at_ms = Some(20);
        }
        let evidence = message_sender::SubmissionEvidence {
            observed_at_ms: 30,
            manager: primitive_types::H256::repeat_byte(4),
            historical_proxy: primitive_types::H256::repeat_byte(6),
            sender: "source-signer".into(),
            manager_reply: Some(vec![0xaa]),
            error: Some("finalized per-log failure".into()),
        };
        manager
            .finalize_transaction(message_sender::Response {
                tx_uuid: uuid,
                status: MessageStatus::RetryableNoop {
                    receipt_key,
                    diagnostic: "finalized per-log failure".into(),
                    receipt_observation: None,
                },
                submission: Some(evidence),
                signed_submission: Some(signed),
            })
            .await
            .unwrap();
        if let TxStatus::RetryableSubmission { retry_at_ms, .. } = &mut manager
            .transactions
            .write()
            .await
            .get_mut(&uuid)
            .unwrap()
            .status
        {
            *retry_at_ms = 0;
        }
        let (requests, mut sent) = unbounded_channel();
        let (_responses, received) = unbounded_channel();
        let mut sender = MessageSenderIo::new(requests, received);
        let (requests, _proof_requests) = unbounded_channel();
        let (_responses, received) = unbounded_channel();
        let mut composer = ProofComposerIo::new(requests, received);
        tokio::time::timeout(
            Duration::from_secs(5),
            manager.resume(&mut sender, &mut composer),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        let original =
            serde_json::to_value(&manager.transactions.read().await[&uuid].receipt).unwrap();
        drop(manager);
        let restored = TransactionManager::new(storage.clone());
        storage.load(&restored).await.unwrap();
        assert_eq!(
            serde_json::to_value(&restored.transactions.read().await[&uuid].receipt).unwrap(),
            original
        );
        restored.resume(&mut sender, &mut composer).await.unwrap();
        assert!(
            matches!(sent.try_recv(), Ok(message_sender::Request::PrepareMessage {
            tx_uuid, payload, ..
        }) if tx_uuid == uuid && payload.encode() == event.encode())
        );
        assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn queued_proof_identity_and_scale_tail_are_checked_before_new_signing() {
        use super::super::storage::JSONStorage;
        let (lock, event) = receipt_fixtures().remove(0);
        for mutation in [
            "trailing SCALE",
            "receipt key",
            "original slot",
            "failed preparation",
            "ambiguous handoff",
        ] {
            let path = std::env::temp_dir().join(format!("inbound-proof-guard-{}", Uuid::now_v7()));
            let storage = Arc::new(JSONStorage::new(&path));
            storage
                .bind_runtime_identity(runtime_identity())
                .await
                .unwrap();
            let manager = TransactionManager::new(storage.clone());
            let transaction = Transaction::new(lock.clone(), TxStatus::ComposeProof);
            let uuid = transaction.uuid;
            manager.add_transaction(transaction).await;
            manager.submit_message(uuid, event.clone()).await.unwrap();
            {
                let mut transactions = manager.transactions.write().await;
                let transaction = transactions.get_mut(&uuid).unwrap();
                match mutation {
                    "trailing SCALE" => transaction.receipt.as_mut().unwrap().payload.push(0xff),
                    "receipt key" => transaction.receipt.as_mut().unwrap().receipt_key.1 += 1,
                    "original slot" => transaction.tx.slot_number.0 += 1,
                    "ambiguous handoff" => {
                        transaction.receipt.as_mut().unwrap().handed_off_at_ms = Some(20)
                    }
                    "failed preparation" => {
                        transaction.status = TxStatus::PreparingSubmission;
                        transaction.receipt.as_mut().unwrap().initial_response =
                            Some(message_sender::SubmissionEvidence {
                                observed_at_ms: 30,
                                manager: primitive_types::H256::repeat_byte(4),
                                historical_proxy: primitive_types::H256::repeat_byte(6),
                                sender: "source-signer".into(),
                                manager_reply: None,
                                error: Some("signing failed before handoff".into()),
                            });
                    }
                    _ => unreachable!(),
                }
            }
            storage.save(&manager).await.unwrap();
            let original = tokio::fs::read(path.join("state.json")).await.unwrap();
            drop(manager);
            let restored = TransactionManager::new(storage.clone());
            let loaded = storage.load(&restored).await;
            if mutation == "failed preparation" || mutation == "ambiguous handoff" {
                loaded.unwrap();
                let (requests, mut sent) = unbounded_channel();
                let (_responses, received) = unbounded_channel();
                let mut sender = MessageSenderIo::new(requests, received);
                let (requests, _proof_requests) = unbounded_channel();
                let (_responses, received) = unbounded_channel();
                let mut composer = ProofComposerIo::new(requests, received);
                restored.resume(&mut sender, &mut composer).await.unwrap();
                if mutation == "ambiguous handoff" {
                    assert!(
                        matches!(sent.try_recv(), Ok(message_sender::Request::ReceiptStatus {
                        tx_uuid, receipt_key,
                    }) if tx_uuid == uuid && receipt_key == (lock.slot_number.0, event.transaction_index))
                    );
                } else {
                    assert!(
                        matches!(sent.try_recv(), Ok(message_sender::Request::PrepareMessage {
                        tx_uuid, payload, ..
                    }) if tx_uuid == uuid && payload.encode() == event.encode())
                    );
                }
                assert!(matches!(sent.try_recv(), Err(TryRecvError::Empty)));
            } else {
                assert!(
                    loaded.is_err(),
                    "untrusted {mutation} must hold before signing"
                );
            }
            assert_eq!(
                tokio::fs::read(path.join("state.json")).await.unwrap(),
                original
            );
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }
}
