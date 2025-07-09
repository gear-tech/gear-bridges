use crate::message_relayer::{common::TxHashWithSlot, eth_to_gear::message_sender::MessageStatus};
use eth_events_electra_client::EthToVaraEvent;
use prometheus::IntCounter;
use sails_rs::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

use super::{
    message_sender::{self, MessageSenderIo},
    proof_composer::{self, ProofComposerIo},
    storage::Storage,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub uuid: Uuid,
    pub status: TxStatus,
    pub tx: TxHashWithSlot,
}

impl Transaction {
    pub fn new(tx: TxHashWithSlot, status: TxStatus) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            status,
            tx,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TxStatus {
    ComposeProof,
    SubmitMessage { payload: Vec<u8> },

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

    async fn update_storage(&self) {
        if let Err(err) = self.storage.save(self).await {
            log::error!("Failed to save transactions to storage: {err:?}");
        }
    }

    pub async fn run(
        self,
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
        mut proof_composer: ProofComposerIo,
        mut message_sender: MessageSenderIo,
    ) -> anyhow::Result<()> {
        if !self
            .resume(&mut message_sender, &mut proof_composer)
            .await?
        {
            // no need to update storage, `resume` does not transition
            // tx status
            return Ok(());
        }

        loop {
            let result = self
                .process(
                    &mut message_paid_events,
                    &mut proof_composer,
                    &mut message_sender,
                )
                .await;
            self.update_storage().await;
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
        tokio::select! {
           value = message_paid_events.recv() =>
               match value {
                   Some(tx) =>  self.compose_proof(tx, proof_composer).await,
                   None => Ok(false)
               },
           value = proof_composer.recv() =>
               match value {
                   Some(proof_composer::Response { tx_uuid, payload }) =>
                       self.submit_message(tx_uuid, payload, message_sender).await,
                   None => Ok(false)
               },

           value = message_sender.recv() =>
               match value {
                   Some(response) => {
                       self.finalize_transaction(response).await?;
                       Ok(true)
                    },
                   None => Ok(false),
               }
        }
    }

    async fn resume(
        &self,
        message_sender: &mut MessageSenderIo,
        proof_composer: &mut ProofComposerIo,
    ) -> anyhow::Result<bool> {
        let transactions = self.transactions.write().await;
        for (_, tx) in transactions.iter() {
            match tx.status {
                TxStatus::ComposeProof => {
                    if !proof_composer.compose_proof_for(tx.uuid, tx.tx.clone()) {
                        log::info!("Proof composer connection closed, exiting...");
                        return Ok(false);
                    }
                }

                TxStatus::SubmitMessage { ref payload } => {
                    let payload = EthToVaraEvent::decode(&mut payload.as_slice())?;

                    if !message_sender.send_message(tx.uuid, tx.tx.tx_hash, payload) {
                        log::info!("Message sender connection closed, exiting...");
                        return Ok(false);
                    }
                }

                TxStatus::Completed => unreachable!(),
            }
        }

        Ok(true)
    }

    async fn compose_proof(
        &self,
        tx: TxHashWithSlot,
        proof_composer: &mut ProofComposerIo,
    ) -> anyhow::Result<bool> {
        let tx = Transaction::new(tx, TxStatus::ComposeProof);

        let tx_uuid = tx.uuid;
        let tx_hash = tx.tx.clone();

        log::info!("Received paid event {tx_hash:?}, transaction UUID: {tx_uuid}");

        self.transactions.write().await.insert(tx_uuid, tx);

        // now that we've seen the transaction it will be saved
        // in regular storage, not in block storage. We can remove
        // it from block storage to avoid doing double work.
        self.storage
            .block_storage()
            .complete_transaction(&tx_hash)
            .await;

        if !proof_composer.compose_proof_for(tx_uuid, tx_hash) {
            log::error!("Proof composer connection closed, exiting...");
            Ok(false)
        } else {
            log::info!("Transaction {tx_uuid} is enqueued for proof composition");
            Ok(true)
        }
    }

    async fn submit_message(
        &self,
        tx_uuid: Uuid,
        payload: EthToVaraEvent,
        message_sender: &mut MessageSenderIo,
    ) -> anyhow::Result<bool> {
        log::info!("Received proof for transaction {tx_uuid}");

        let mut transactions = self.transactions.write().await;

        match transactions.get_mut(&tx_uuid) {
            Some(tx) => {
                if let TxStatus::ComposeProof = tx.status {
                    tx.status = TxStatus::SubmitMessage {
                        payload: payload.encode(),
                    };
                    let tx_hash = tx.tx.tx_hash;
                    let tx_uuid = tx.uuid;

                    drop(transactions);

                    if !message_sender.send_message(tx_uuid, tx_hash, payload) {
                        log::info!("Message sender connection terminated, exiting...");
                        return Ok(false);
                    }
                } else {
                    log::warn!(
                        "Received proof for a transaction that is not in ComposeProof state"
                    );
                }
            }

            None => {
                log::warn!("Received proof for unknown transaction: {tx_uuid}");
            }
        }

        Ok(true)
    }

    async fn finalize_transaction(&self, response: message_sender::Response) -> anyhow::Result<()> {
        let message_sender::Response { tx_uuid, status } = response;

        log::info!("Received response for transaction {tx_uuid}: {status:?}");
        let mut transactions = self.transactions.write().await;

        if let Some(mut tx) = transactions.remove(&tx_uuid) {
            match status {
                MessageStatus::Success => {
                    tx.status = TxStatus::Completed;
                    // transaction may have been failed and restarted. Remove
                    // it from failed set if it succeeded.
                    self.failed.write().await.remove(&tx.uuid);
                    self.completed.write().await.insert(tx.uuid, tx);
                    self.metrics.completed_transactions.inc();
                }

                MessageStatus::Failure(message) => {
                    self.fail_transaction(tx_uuid, message).await;
                    // add transaction back to tx queue. It can be restarted later.
                    transactions.insert(tx_uuid, tx);
                }
            }
        } else {
            log::warn!("Received response for unknown transaction {tx_uuid}");
        }

        Ok(())
    }
}
