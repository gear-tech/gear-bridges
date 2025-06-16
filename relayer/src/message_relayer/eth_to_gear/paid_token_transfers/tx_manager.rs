use sails_rs::Encode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use uuid::Uuid;

use crate::message_relayer::{
    common::TxHashWithSlot, eth_to_gear::paid_token_transfers::message_sender::SendStatus,
};

use super::{
    message_sender::{MessageSenderIo, Response},
    proof_composer::{ComposedProof, ProofComposerIo},
    storage::{NoStorage, Storage},
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
    Failed(String),
}

pub struct TransactionManager {
    /// Queue of transactions to be processed. Completed and failed
    /// transactions are moved to `completed` and `failed` maps.
    pub transactions: RwLock<BTreeMap<Uuid, Transaction>>,

    pub completed: RwLock<BTreeMap<Uuid, Transaction>>,
    pub failed: RwLock<BTreeMap<Uuid, Transaction>>,

    /// Do we need to restart failed transactions? At the moment this only applies
    /// to failed transactions loaded from storage.
    pub restart_failed: bool,
    pub resume_from_storage: bool,
    pub storage: Box<dyn Storage>,
}

impl TransactionManager {
    pub fn new(
        restart_failed: bool,
        resume_from_storage: bool,
        storage: Option<Box<dyn Storage>>,
    ) -> Self {
        Self {
            transactions: RwLock::new(BTreeMap::new()),
            completed: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),

            restart_failed,
            resume_from_storage,
            storage: storage.unwrap_or_else(|| Box::new(NoStorage)),
        }
    }

    pub async fn add_transaction(&self, tx: Transaction) {
        match tx.status {
            TxStatus::Completed => {
                self.completed.write().await.insert(tx.uuid, tx);
            }
            // if `restart_failed` is false just keep the tx here.
            TxStatus::Failed(_) if !self.restart_failed => {
                self.failed.write().await.insert(tx.uuid, tx);
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

    async fn read_from_storage(&self) {
        if let Err(err) = self.storage.load(self).await {
            log::error!("Failed to load transactions from storage: {err:?}");
        }
    }

    pub async fn run(
        self,
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
        mut proof_composer: ProofComposerIo,
        mut message_sender: MessageSenderIo,
    ) -> anyhow::Result<()> {
        self.read_from_storage().await;
        'exit: {
            if self.resume_from_storage {
                let mut transactions = self.transactions.write().await;
                for (_, tx) in transactions.iter() {
                    match tx.status {
                        TxStatus::ComposeProof => {
                            if !proof_composer.compose_proof_for(tx.uuid, tx.tx.clone()) {
                                log::info!("Proof composer connection closed, exiting...");
                                break 'exit;
                            }
                        }

                        TxStatus::Completed | TxStatus::Failed(_) => unreachable!(),
                        _ => todo!(),
                    }
                }

                if self.restart_failed {
                    let mut failed = self.failed.write().await;

                    while let Some((tx_uuid, tx)) = failed.pop_last() {
                        debug_assert!(matches!(tx.status, TxStatus::Failed(_)));
                        log::info!("Restarting failed transaction {tx_uuid}: {:?}", tx.tx);
                        transactions.insert(tx_uuid, tx);
                    }
                }
            }

            loop {
                tokio::select! {
                    Some(tx) = message_paid_events.recv() => {
                        let tx = Transaction::new(tx.clone(), TxStatus::ComposeProof);
                        let tx_uuid = tx.uuid;
                        let tx_hash = tx.tx.clone();
                        log::info!("Received paid event {tx_hash:?}, transaction UUID: {tx_uuid}");
                        self.transactions.write().await.insert(tx_uuid, tx);
                        if !proof_composer.compose_proof_for(tx_uuid, tx_hash) {
                            log::error!("Proof composer connection closed, exiting...");
                            break 'exit;
                        } else {
                            log::info!("Transaction {tx_uuid} is enqueued for proof composition");
                        }

                    }

                    Some(ComposedProof { payload, tx_uuid }) = proof_composer.recv() => {
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

                                    self.update_storage().await;
                                    if !message_sender.send_message(tx_uuid, tx_hash, payload) {
                                        log::info!("Message sender connection terminated, exiting...");
                                        break 'exit;
                                    }
                                } else {
                                    log::warn!("Received proof for a transaction that is not in ComposeProof state");
                                }
                            }

                            None => {
                                log::warn!("Received proof for unknown transaction: {tx_uuid}");
                            }
                        }
                    }

                    Some(Response { tx_uuid, status }) = message_sender.receive_response() => {
                        log::info!("Received response for transaction {tx_uuid}: {status:?}");

                        let mut transactions = self.transactions.write().await;

                        if let Some(mut tx) = transactions.remove(&tx_uuid) {
                            match status {
                                SendStatus::Success => {
                                    tx.status = TxStatus::Completed;
                                    self.completed.write()
                                        .await
                                        .insert(tx.uuid, tx);
                                }

                                SendStatus::Failure(message) => {
                                    tx.status = TxStatus::Failed(message);
                                    self.failed
                                        .write()
                                        .await
                                        .insert(tx.uuid, tx);
                                }
                            }
                        } else {
                            log::warn!("Received response for unknown transaction {tx_uuid}");
                        }
                    }

                    else => {
                        log::info!("One of connections terminated, exiting...");
                        break 'exit;
                    }
                }
            }
        }

        self.update_storage().await;

        Ok(())
    }
}
