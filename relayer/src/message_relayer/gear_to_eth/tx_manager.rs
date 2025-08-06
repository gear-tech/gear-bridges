use ethereum_client::TxHash;
use gear_rpc_client::dto::{MerkleProof, Message};
use keccak_hash::keccak_256;
use prometheus::IntCounter;
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
        MessageInBlock, RelayedMerkleRoot,
    },
    gear_to_eth::storage::Storage,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub uuid: Uuid,
    pub message: MessageInBlock,
    pub message_hash: [u8; 32],
    pub status: TxStatus,
}

impl Transaction {
    pub fn new(message: MessageInBlock, status: TxStatus) -> Self {
        let uuid = Uuid::now_v7();
        Self {
            uuid,
            status,
            message_hash: message_hash(&message.message),
            message,
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

    pub async fn add_transaction(&self, tx: Transaction) {
        self.metrics.total_transactions.inc();

        match tx.status {
            TxStatus::Completed => {
                self.completed.write().await.insert(tx.uuid, tx.clone());
                self.metrics.completed_transactions.inc();
            }

            _ => {
                self.transactions.write().await.insert(tx.uuid, tx);
            }
        }
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
                    log::info!(
                        "Transaction {}, nonce={} is waiting for merkle root",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_le)
                    );
                    if !accumulator.send_message(
                        tx.uuid,
                        tx.message.authority_set_id,
                        tx.message.block,
                        tx.message.block_hash,
                    ) {
                        log::warn!("Accumulator stopped accepting messages, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::FetchMerkleRoot(ref merkle_root) => {
                    log::info!(
                        "Transaction {}, nonce={} is fetching merkle root for block #{}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_le),
                        merkle_root.block
                    );
                    if !proof_fetcher.send_request(
                        tx.uuid,
                        tx.message_hash,
                        tx.message.message.nonce_le,
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
                        hex::encode(tx.message.message.nonce_le),
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
                        hex::encode(tx.message.message.nonce_le),
                        tx_hash
                    );
                    if !status_fetcher.send_request(tx.uuid, tx_hash) {
                        log::warn!("Status fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::Completed => {
                    // Completed transactions do not need to be resumed
                    continue;
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

                let tx = Transaction::new(message, TxStatus::WaitForMerkleRoot);

                let block_hash = tx.message.block_hash;
                let authority_set_id = tx.message.authority_set_id;
                let block = tx.message.block;
                let uuid = tx.uuid;

                self.add_transaction(tx).await;

                if !accumulator.send_message(uuid, authority_set_id, block, block_hash) {
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
                            if !proof_fetcher.send_request(
                                tx_uuid,
                                tx.message_hash,
                                tx.message.message.nonce_le,
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
                        self.fail_transaction(message.tx_uuid, "Message overflowed".to_string()).await;
                    }

                    accumulator::Response::Stuck { tx_uuid, .. } => {

                        self.fail_transaction(tx_uuid, "Message stuck".to_string()).await;
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
                            "Message already processed, skipping: tx_uuid = {tx_uuid}"
                        );
                        if let Some(tx) = self.transactions.write().await.get_mut(&tx_uuid) {
                            tx.status = TxStatus::Completed;
                        } else {
                            log::warn!("Received message for unknown transaction: {tx_uuid}");
                        }

                    }

                    message_sender::Response::ProcessingStarted(tx_hash, tx_uuid) => {
                        if let Some(tx) = self.transactions.write().await.get_mut(&tx_uuid) {
                            tx.status = TxStatus::WaitConfirmations(tx_hash);
                        } else {
                            log::warn!("Received message for unknown transaction: {tx_uuid}");
                        }

                        if !status_fetcher.send_request(tx_uuid, tx_hash) {
                            log::warn!("Status fetcher stopped accepting requests, exiting");
                            return Ok(false);
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
                        if let Some(tx) = self.transactions.write().await.remove(&uuid) {
                            let completed_tx = Transaction {
                                uuid,
                                message: tx.message,
                                message_hash: tx.message_hash,
                                status: TxStatus::Completed,
                            };
                            self.completed.write().await.insert(uuid, completed_tx);
                            self.metrics.completed_transactions.inc();

                            log::info!(
                                "Transaction {uuid} completed successfully: tx_hash = {tx_hash}"
                            );
                        } else {
                            log::warn!("Received success response for unknown transaction: {uuid}");
                        }
                    }

                    status_fetcher::Response::Failed(uuid, e) => {
                        if let Some(tx) = self.transactions.write().await.remove(&uuid) {
                            self.fail_transaction(uuid, e.to_string()).await;
                            let nonce = hex::encode(tx.message.message.nonce_le);
                            log::error!("Transaction {uuid}, nonce={nonce} failed: {e}", );
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

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}
