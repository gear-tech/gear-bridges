use ethereum_client::TxHash;
use gear_rpc_client::dto::MerkleProof;
use prometheus::IntCounter;
use sails_rs::ActorId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::{self, utils::MerkleRoots, AccumulatorIo},
            message_sender::{self, MessageSenderIo},
            status_fetcher::{self, StatusFetcherIo},
        },
        gear::merkle_proof_fetcher::MerkleRootFetcherIo,
        message_hash, MessageInBlock, RelayedMerkleRoot,
    },
    gear_to_eth::storage::Storage,
};

const MAX_DROPPED_RETRIES: u32 = 3;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub uuid: Uuid,
    pub message: MessageInBlock,
    pub message_hash: [u8; 32],
    pub status: TxStatus,
    #[serde(default)]
    pub dropped_retries: u32,
}

impl Transaction {
    pub fn new(message: MessageInBlock, status: TxStatus) -> Self {
        let uuid = Uuid::now_v7();
        Self {
            uuid,
            status,
            message_hash: message_hash(&message.message),
            message,
            dropped_retries: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TxStatus {
    WaitForMerkleRoot,
    FetchMerkleRoot(RelayedMerkleRoot),
    SendMessage(RelayedMerkleRoot, MerkleProof),
    WaitConfirmations(TxHash),
    Completed,
}

enum ExistingTransaction {
    Completed,
    Active(Box<Transaction>),
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
    pub merkle_roots: Arc<RwLock<MerkleRoots>>,

    pub transactions: RwLock<BTreeMap<Uuid, Transaction>>,
    pub failed: RwLock<BTreeMap<Uuid, String>>,
    pub completed: RwLock<BTreeMap<Uuid, Transaction>>,
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
            merkle_roots: Arc::new(RwLock::new(MerkleRoots::new(100))),
            transactions: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),
            completed: RwLock::new(BTreeMap::new()),
            storage,

            metrics: Metrics::new(),
        }
    }

    pub async fn fail_transaction(&self, tx_uuid: Uuid, reason: String) {
        self.failed.write().await.insert(tx_uuid, reason);
        self.metrics.failed_transactions.inc();
    }

    async fn fail_active_transaction(&self, tx_uuid: Uuid, reason: String) -> Option<Transaction> {
        let tx = self.transactions.write().await.remove(&tx_uuid);
        if tx.is_some() {
            self.fail_transaction(tx_uuid, reason).await;
        }
        tx
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

    async fn complete_transaction(&self, uuid: Uuid) -> bool {
        let tx = self.transactions.write().await.remove(&uuid);
        let Some(mut tx) = tx else {
            return false;
        };

        tx.status = TxStatus::Completed;
        self.completed.write().await.insert(uuid, tx);
        self.failed.write().await.remove(&uuid);
        self.metrics.completed_transactions.inc();
        true
    }

    async fn existing_transaction(&self, message_hash: [u8; 32]) -> Option<ExistingTransaction> {
        if self
            .completed
            .read()
            .await
            .values()
            .any(|tx| tx.message_hash == message_hash)
        {
            return Some(ExistingTransaction::Completed);
        }

        self.transactions
            .read()
            .await
            .values()
            .find(|tx| tx.message_hash == message_hash)
            .cloned()
            .map(Box::new)
            .map(ExistingTransaction::Active)
    }

    pub async fn update_storage(&self) {
        if let Err(err) = self.storage.save(self).await {
            log::error!("Failed to save transaction manager state: {err}");
        }
    }

    pub async fn load_from_storage(&self) -> anyhow::Result<()> {
        self.storage.load(self).await
    }

    async fn resume(
        &self,
        accumulator: &mut AccumulatorIo,
        proof_fetcher: &mut MerkleRootFetcherIo,
        message_sender: &mut MessageSenderIo,
        status_fetcher: &mut StatusFetcherIo,
    ) -> anyhow::Result<bool> {
        let transactions = self.transactions.write().await;

        for (_, tx) in transactions.iter() {
            match tx.status {
                TxStatus::WaitForMerkleRoot => {
                    self.storage
                        .block_storage()
                        .complete_transaction(&tx.message)
                        .await;
                    log::info!(
                        "Transaction {}, nonce={} is waiting for merkle root",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be)
                    );
                    if !accumulator.send_message(
                        tx.uuid,
                        tx.message.authority_set_id,
                        tx.message.block,
                        tx.message.block_hash,
                        ActorId::from(tx.message.message.source),
                    ) {
                        log::warn!("Accumulator stopped accepting messages, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::FetchMerkleRoot(ref merkle_root) => {
                    log::info!(
                        "Transaction {}, nonce={} is fetching merkle root for block #{}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be),
                        merkle_root.block
                    );
                    if !proof_fetcher.send_request(
                        tx.uuid,
                        tx.message.block.0,
                        tx.message_hash,
                        tx.message.message.nonce_be,
                        *merkle_root,
                    ) {
                        log::warn!("Merkle root fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::SendMessage(ref relayed_merkle_root, ref proof) => {
                    log::info!(
                        "Transaction {}, nonce={} is being relayed with merkle root for block #{}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be),
                        relayed_merkle_root.block
                    );
                    if !message_sender.send(
                        tx.message.message.clone(),
                        *relayed_merkle_root,
                        proof.clone(),
                        tx.uuid,
                    ) {
                        log::warn!("Message sender stopped accepting messages, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::WaitConfirmations(tx_hash) => {
                    log::info!(
                        "Transaction {}, nonce={} is waiting for confirmations, tx_hash={}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be),
                        tx_hash
                    );
                    if !status_fetcher.send_request_with_bridge_nonce(
                        tx.uuid,
                        tx_hash,
                        tx.message.message.nonce_be,
                    ) {
                        log::warn!("Status fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::Completed => {
                    // Completed transactions do not need to be resumed
                    // no-op
                }
            }
        }

        Ok(true)
    }

    pub async fn run(
        self,
        mut accumulator: AccumulatorIo,
        mut queued_messages: UnboundedReceiver<MessageInBlock>,
        mut proof_fetcher: MerkleRootFetcherIo,
        mut message_sender: MessageSenderIo,
        mut status_fetcher: StatusFetcherIo,
    ) -> anyhow::Result<()> {
        if !self
            .resume(
                &mut accumulator,
                &mut proof_fetcher,
                &mut message_sender,
                &mut status_fetcher,
            )
            .await?
        {
            log::warn!("Failed to resume transaction manager, exiting");
            return Ok(());
        }

        loop {
            let result = self
                .process(
                    &mut accumulator,
                    &mut queued_messages,
                    &mut proof_fetcher,
                    &mut message_sender,
                    &mut status_fetcher,
                )
                .await;

            self.update_storage().await;

            match result {
                Ok(false) => {
                    log::warn!("No new transactions to process, exiting");
                    break;
                }

                Ok(true) => continue,
                Err(err) => {
                    log::error!("Transaction manager error: {err}");
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub async fn process(
        &self,
        accumulator: &mut AccumulatorIo,
        queued_messages: &mut UnboundedReceiver<MessageInBlock>,
        proof_fetcher: &mut MerkleRootFetcherIo,
        message_sender: &mut MessageSenderIo,
        status_fetcher: &mut StatusFetcherIo,
    ) -> anyhow::Result<bool> {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("Received Ctrl+C signal, exiting");
                return Ok(false);
            }

            message = queued_messages.recv() => {
                let Some(message) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(false);
                };

                let hash = message_hash(&message.message);
                match self.existing_transaction(hash).await {
                    Some(ExistingTransaction::Completed) => {
                        log::info!(
                            "Skipping completed message: nonce={}, block=#{}",
                            hex::encode(message.message.nonce_be),
                            message.block.0
                        );
                        return Ok(true);
                    }
                    Some(ExistingTransaction::Active(tx)) => {
                        log::info!(
                            "Skipping active message: transaction={}, status={:?}, nonce={}, block=#{}",
                            tx.uuid,
                            tx.status,
                            hex::encode(tx.message.message.nonce_be),
                            tx.message.block.0,
                        );
                        return Ok(true);
                    }
                    None => {}
                }

                self.storage.block_storage().complete_transaction(&message).await;
                let tx = Transaction::new(message, TxStatus::WaitForMerkleRoot);
                self.add_transaction(tx.clone()).await;

                let source = ActorId::from(tx.message.message.source);
                let block_hash = tx.message.block_hash;
                let authority_set_id = tx.message.authority_set_id;
                let block = tx.message.block;
                let uuid = tx.uuid;

                log::info!(
                    "New transaction {uuid}, nonce={} from source {source} at block #{} ({block_hash})",
                    hex::encode(tx.message.message.nonce_be),
                    block.0,
                );

                if !accumulator.send_message(uuid, authority_set_id, block, block_hash, source) {
                    log::warn!("Failed to send message to accumulator, exiting");
                    return Ok(false);
                }
            }

            message = accumulator.recv_message() => {
                let Some(message) = message else {
                    log::info!("No more messages from accumulator, exiting");
                    return Ok(false);
                };

                match message {
                    accumulator::Response::Success { tx_uuid, merkle_root, ..} => {
                        if let Some(tx) = self.transactions.write().await.get_mut(&tx_uuid) {
                            tx.status = TxStatus::FetchMerkleRoot(merkle_root);
                            log::info!(
                                "Transaction {} at block #{}({}), hash={}, nonce={} got merkle root {} for block #{}",
                                tx_uuid,
                                tx.message.block.0,
                                tx.message.block_hash,
                                hex::encode(message_hash(&tx.message.message)),
                                hex::encode(tx.message.message.nonce_be),
                                merkle_root.merkle_root,
                                merkle_root.block
                            );
                            if !proof_fetcher.send_request(
                                tx_uuid,
                                tx.message.block.0,
                                tx.message_hash,
                                tx.message.message.nonce_be,
                                merkle_root,
                            ) {
                                log::warn!("Merkle root fetcher stopped accepting requests, exiting");
                                return Ok(false);
                            }
                        } else {
                            log::warn!("Received success response for unknown transaction: {tx_uuid}");
                        }
                    }

                    accumulator::Response::Overflowed(message) => {
                        if self
                            .fail_active_transaction(
                                message.tx_uuid,
                                "Message overflowed".to_string(),
                            )
                            .await
                            .is_none()
                        {
                            log::warn!(
                                "Received overflow response for unknown transaction: {}",
                                message.tx_uuid
                            );
                        }
                    }

                    accumulator::Response::Stuck { tx_uuid, .. } => {
                        if self
                            .fail_active_transaction(tx_uuid, "Message stuck".to_string())
                            .await
                            .is_none()
                        {
                            log::warn!("Received stuck response for unknown transaction: {tx_uuid}");
                        }
                    }
                }
            }

            message = proof_fetcher.recv_message() => {
                let Some(message) = message else {
                    log::info!("No more messages from proof fetcher, exiting");
                    return Ok(false);
                };

                if let Some(tx) = self.transactions.write().await.get_mut(&message.tx_uuid) {
                    tx.status = TxStatus::SendMessage(
                        message.merkle_root,
                        message.proof.clone(),
                    );

                    if !message_sender.send(
                        tx.message.message.clone(),
                        message.merkle_root,
                        message.proof,
                        tx.uuid,
                    ) {
                        log::warn!("Message sender stopped accepting messages, exiting");
                        return Ok(false);
                    }
                } else {
                    log::warn!("Received merkle proof for unknown transaction: {}", message.tx_uuid);
                }
            }

            message = message_sender.recv() => {
                let Some(message) = message else {
                    log::info!("No more messages from message sender, exiting");
                    return Ok(false);
                };

                match message {
                    message_sender::Response::MessageAlreadyProcessed(tx_uuid) => {
                        log::info!(
                            "Message already processed, completing: tx_uuid = {tx_uuid}"
                        );
                        if !self.complete_transaction(tx_uuid).await {
                            log::warn!("Received message for unknown transaction: {tx_uuid}");
                        }
                    }

                    message_sender::Response::ProcessingStarted(tx_hash, tx_uuid) => {
                        let bridge_nonce = if let Some(tx) =
                            self.transactions.write().await.get_mut(&tx_uuid)
                        {
                            tx.status = TxStatus::WaitConfirmations(tx_hash);
                            Some(tx.message.message.nonce_be)
                        } else {
                            log::warn!("Received message for unknown transaction: {tx_uuid}");
                            None
                        };

                        if let Some(bridge_nonce) = bridge_nonce {
                            if !status_fetcher.send_request_with_bridge_nonce(tx_uuid, tx_hash, bridge_nonce) {
                                log::warn!("Status fetcher stopped accepting requests, exiting");
                                return Ok(false);
                            }
                        }
                    }

                    message_sender::Response::Failed(tx_uuid, error) => {
                        if let Some(tx) = self
                            .fail_active_transaction(tx_uuid, error.clone())
                            .await
                        {
                            let nonce = hex::encode(tx.message.message.nonce_be);
                            log::error!(
                                "Transaction {tx_uuid}, nonce={nonce} failed before broadcast: {error}"
                            );
                        } else {
                            log::warn!("Received failure for unknown transaction: {tx_uuid}");
                        }
                    }
                }
            }

            message = status_fetcher.recv_message() => {
                let Some(status) = message else {
                    log::info!("No more messages from status fetcher, exiting");
                    return Ok(false);
                };

                match status {
                    status_fetcher::Response::Success(uuid, tx_hash) => {
                        if self.complete_transaction(uuid).await {
                            log::info!(
                                "Transaction {uuid} completed successfully: tx_hash = {tx_hash}"
                            );
                        } else {
                            log::warn!("Received success response for unknown transaction: {uuid}");
                        }
                    }

                    status_fetcher::Response::Dropped(uuid, error) => {
                        let retry = {
                            let mut transactions = self.transactions.write().await;
                            transactions.get_mut(&uuid).map(|tx| {
                                tx.dropped_retries = tx.dropped_retries.saturating_add(1);
                                tx.status = TxStatus::WaitForMerkleRoot;
                                (
                                    tx.dropped_retries,
                                    tx.message.authority_set_id,
                                    tx.message.block,
                                    tx.message.block_hash,
                                    ActorId::from(tx.message.message.source),
                                    hex::encode(tx.message.message.nonce_be),
                                )
                            })
                        };

                        if let Some((attempt, authority_set_id, block, block_hash, source, nonce)) =
                            retry
                        {
                            if attempt > MAX_DROPPED_RETRIES {
                                let reason = format!(
                                    "Ethereum transaction repeatedly disappeared after {MAX_DROPPED_RETRIES} retries: {error}"
                                );
                                let _ = self.fail_active_transaction(uuid, reason.clone()).await;
                                log::error!(
                                    "Transaction {uuid}, nonce={nonce} failed terminally: {reason}"
                                );
                            } else {
                                log::error!(
                                    "Transaction {uuid}, nonce={nonce} was dropped: {error}. Retrying from merkle-root lookup ({attempt}/{MAX_DROPPED_RETRIES})"
                                );
                                if !accumulator.send_message(
                                    uuid,
                                    authority_set_id,
                                    block,
                                    block_hash,
                                    source,
                                ) {
                                    log::warn!("Accumulator stopped accepting messages, exiting");
                                    return Ok(false);
                                }
                            }
                        } else {
                            log::warn!("Received dropped response for unknown transaction: {uuid}");
                        }
                    }

                    status_fetcher::Response::Failed(uuid, error) => {
                        if let Some(tx) =
                            self.fail_active_transaction(uuid, error.clone()).await
                        {
                            let nonce = hex::encode(tx.message.message.nonce_be);
                            log::error!(
                                "Transaction {uuid}, nonce={nonce} failed terminally: {error}"
                            );
                        } else {
                            log::warn!("Received failure response for unknown transaction: {uuid}");
                        }
                    }
                }
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::{
        common::{AuthoritySetId, GearBlockNumber},
        gear_to_eth::storage::NoStorage,
    };
    use gear_rpc_client::dto::Message;
    use primitive_types::H256;

    fn transaction() -> Transaction {
        Transaction::new(
            MessageInBlock {
                message: Message {
                    nonce_be: [7; 32],
                    source: [1; 32],
                    destination: [2; 20],
                    payload: vec![3, 4],
                },
                block: GearBlockNumber(42),
                block_hash: H256::from_low_u64_be(99),
                authority_set_id: AuthoritySetId(5),
            },
            TxStatus::WaitForMerkleRoot,
        )
    }

    #[tokio::test]
    async fn complete_transaction_moves_active_transaction_to_completed() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let tx = transaction();
        let uuid = tx.uuid;
        let message_hash = tx.message_hash;

        manager.add_transaction(tx).await;
        manager
            .failed
            .write()
            .await
            .insert(uuid, "previous attempt failed".to_string());

        assert!(manager.complete_transaction(uuid).await);
        assert!(!manager.transactions.read().await.contains_key(&uuid));
        assert!(!manager.failed.read().await.contains_key(&uuid));

        let completed = manager.completed.read().await;
        let completed_tx = completed.get(&uuid).unwrap();
        assert_eq!(completed_tx.message_hash, message_hash);
        assert!(matches!(completed_tx.status, TxStatus::Completed));
        drop(completed);

        assert!(!manager.complete_transaction(uuid).await);
        assert_eq!(manager.metrics.completed_transactions.get(), 1);
    }

    #[tokio::test]
    async fn existing_transactions_are_classified_for_duplicate_suppression() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let tx = transaction();
        let uuid = tx.uuid;
        let message_hash = tx.message_hash;

        manager.add_transaction(tx).await;
        assert!(matches!(
            manager.existing_transaction(message_hash).await,
            Some(ExistingTransaction::Active(_))
        ));

        assert!(manager.complete_transaction(uuid).await);
        assert!(matches!(
            manager.existing_transaction(message_hash).await,
            Some(ExistingTransaction::Completed)
        ));
        assert!(manager.existing_transaction([0; 32]).await.is_none());
    }

    #[test]
    fn legacy_transaction_state_defaults_dropped_retry_count() {
        let tx = transaction();
        let mut value = serde_json::to_value(tx).unwrap();
        value.as_object_mut().unwrap().remove("dropped_retries");

        let restored: Transaction = serde_json::from_value(value).unwrap();
        assert_eq!(restored.dropped_retries, 0);
    }

    #[tokio::test]
    async fn terminal_failure_removes_active_transaction_for_explicit_retry() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let tx = transaction();
        let uuid = tx.uuid;
        let message_hash = tx.message_hash;
        manager.add_transaction(tx).await;

        assert!(manager
            .fail_active_transaction(uuid, "terminal failure".to_string())
            .await
            .is_some());
        assert!(manager.existing_transaction(message_hash).await.is_none());
        assert_eq!(
            manager.failed.read().await.get(&uuid).map(String::as_str),
            Some("terminal failure")
        );
    }

    #[tokio::test]
    async fn every_active_status_suppresses_duplicate_submission() {
        let root = RelayedMerkleRoot {
            block: GearBlockNumber(43),
            block_hash: H256::from_low_u64_be(100),
            timestamp: 123,
            authority_set_id: AuthoritySetId(6),
            merkle_root: H256::from_low_u64_be(101),
        };
        let proof = MerkleProof {
            root: [0; 32],
            proof: vec![],
            num_leaves: 1,
            leaf_index: 0,
        };
        let statuses = [
            TxStatus::WaitForMerkleRoot,
            TxStatus::FetchMerkleRoot(root),
            TxStatus::SendMessage(root, proof),
            TxStatus::WaitConfirmations(TxHash::from([0; 32])),
            TxStatus::Completed,
        ];

        for status in statuses {
            let manager = TransactionManager::new(Arc::new(NoStorage::new()));
            let mut tx = transaction();
            tx.status = status;
            let message_hash = tx.message_hash;
            manager.transactions.write().await.insert(tx.uuid, tx);

            assert!(matches!(
                manager.existing_transaction(message_hash).await,
                Some(ExistingTransaction::Active(_))
            ));
        }
    }
}
