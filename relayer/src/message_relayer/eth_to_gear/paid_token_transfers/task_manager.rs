use eth_events_electra_client::EthToVaraEvent;
use parity_scale_codec::DecodeAll;
use sails_rs::Encode;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

use crate::message_relayer::common::TxHashWithSlot;
use crate::message_relayer::eth_to_gear::paid_token_transfers::message_sender::Message;
use crate::message_relayer::eth_to_gear::paid_token_transfers::message_sender::Response;
use crate::message_relayer::eth_to_gear::paid_token_transfers::message_sender::SendStatus;
use crate::message_relayer::eth_to_gear::paid_token_transfers::proof_composer::ProofComposerRequest;
use crate::message_relayer::eth_to_gear::paid_token_transfers::proof_composer::ProofComposerResponse;
use crate::message_relayer::eth_to_gear::paid_token_transfers::storage::Storage;

pub struct TaskManager {
    /// Queue of tasks to be processed given
    /// that their dependencies are met.
    pub task_queue: RwLock<BTreeMap<Uuid, Task>>,
    pub failed: RwLock<BTreeMap<Uuid, Task>>,
    pub completed: RwLock<BTreeMap<Uuid, Task>>,

    pub storage: Storage,
}

impl TaskManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(storage: Storage) -> Arc<Self> {
        Arc::new(Self {
            task_queue: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),
            completed: RwLock::new(BTreeMap::new()),

            storage,
        })
    }

    pub async fn update_storage(&self) {
        let tasks = self.task_queue.read().await;
        if let Err(err) = self.storage.save_tasks(&tasks).await {
            log::error!("Failed to save tasks to storage: {err}");
        }
    }

    pub async fn read_storage(&self) -> anyhow::Result<BTreeMap<Uuid, Task>> {
        self.storage
            .load_tasks()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to load tasks from storage: {err}"))
    }

    pub async fn run(
        self: Arc<Self>,
        resume_from_storage: bool,
        (proof_tx, mut proof_rx): (
            UnboundedSender<ProofComposerRequest>,
            UnboundedReceiver<ProofComposerResponse>,
        ),
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
        (message_sender, mut response_receiver): (
            UnboundedSender<Message>,
            UnboundedReceiver<Response>,
        ),
    ) -> anyhow::Result<()> {
        if let Ok(tasks) = self.read_storage().await {
            let mut queue = self.task_queue.write().await;
            for (uuid, task) in tasks {
                queue.insert(uuid, task);
            }
        } else {
            log::warn!("Failed to read tasks from storage, starting with an empty task queue.");
        }

        if resume_from_storage {
            match self.read_storage().await {
                Ok(tasks) => {
                    let mut task_queue = self.task_queue.write().await;
                    let mut failed = self.failed.write().await;
                    let mut completed = self.completed.write().await;
                    for (uuid, mut task) in tasks.into_iter() {
                        match task.state {
                            TaskState::ComposeProof => {
                                proof_tx
                                    .send(ProofComposerRequest {
                                        task_uuid: task.uuid,
                                        tx: task.tx.clone(),
                                    })
                                    .expect("Failed to send proof composer request");
                                log::info!("Resumed task {} with ComposeProof state.", task.uuid);
                                task_queue.insert(uuid, task);
                            }

                            TaskState::SubmitMessage { ref payload } => {
                                if let Ok(payload) =
                                    EthToVaraEvent::decode_all(&mut payload.as_slice())
                                {
                                    message_sender
                                        .send(Message {
                                            task_uuid: task.uuid,
                                            payload,
                                            tx_hash: task.tx.tx_hash,
                                        })
                                        .expect("Failed to send message");
                                    log::info!(
                                        "Resumed task {} with SubmitMessage state.",
                                        task.uuid
                                    );
                                    task_queue.insert(uuid, task);
                                } else {
                                    log::warn!("Failed to decode payload for task {} in SubmitMessage state.", task.uuid);
                                    task.state =
                                        TaskState::Failed("Failed to decode payload".to_string());
                                    failed.insert(uuid, task);
                                }
                            }

                            TaskState::Completed => {
                                log::info!("Resumed task {} with Completed state.", task.uuid);
                                completed.insert(uuid, task);
                            }
                            TaskState::Failed(_) => {
                                log::info!("Resumed task {} with Failed state.", task.uuid);
                                failed.insert(uuid, task);
                            }
                        }
                    }
                }

                Err(err) => {
                    log::warn!("Failed to read tasks from storage: {err}. Starting with an empty task queue.");
                }
            }
        }

        loop {
            loop {
                tokio::select! {
                    Some(tx) = message_paid_events.recv() => {
                        log::info!("Received paid event: {tx:?}");
                        let task = Task::new(tx.clone(), TaskState::ComposeProof);
                        let uuid = task.uuid;
                        self.task_queue.write().await.insert(task.uuid, task);
                        self.update_storage().await;
                        proof_tx.send(ProofComposerRequest {
                            task_uuid: uuid,
                            tx,
                        }).expect("Failed to send proof composer request");
                        log::info!("Enqueued task {uuid:?} for proof composition.");

                    }

                    Some(ProofComposerResponse { task_uuid, payload }) = proof_rx.recv() => {
                        log::info!("Received proof for task {task_uuid:?}");
                        let mut tasks = self.task_queue.write().await;
                        match tasks.get_mut(&task_uuid) {
                            Some(task) => {
                                if let TaskState::ComposeProof = &task.state {
                                    if let Err(err) = message_sender.send(Message {
                                        task_uuid,
                                        payload: payload.clone(),
                                        tx_hash: task.tx.tx_hash,
                                    }) {
                                        log::error!("Failed to send message for task {task_uuid:?}: {err}");
                                        continue;
                                    }
                                    task.state = TaskState::SubmitMessage {
                                        payload: payload.encode(),
                                    };
                                    log::info!("Task {task_uuid:?} state updated to SubmitMessage.");
                                } else {
                                    log::warn!("Received proof for task {task_uuid:?} that is not in ComposeProof state.");
                                }
                            }
                            None => {
                                log::warn!("Received proof for unknown task {task_uuid:?}");
                            }
                        }
                    }

                    Some(Response { task_uuid, status }) = response_receiver.recv() => {
                        log::info!("Received response for task {task_uuid:?}: {status:?}");
                        let mut tasks = self.task_queue.write().await;
                        if let Some(mut task) = tasks.remove(&task_uuid) {
                            match status {
                                SendStatus::Success => {
                                    task.state = TaskState::Completed;
                                    self.completed.write().await.insert(task_uuid, task);
                                    log::info!("Task {task_uuid:?} completed successfully.");
                                }
                                SendStatus::Failure(err) => {
                                    task.state = TaskState::Failed(err.clone());
                                    self.failed.write().await.insert(task_uuid, task);
                                    log::error!("Task {task_uuid:?} failed: {err}");
                                }
                            }
                        } else {
                            log::warn!("Received response for unknown task {task_uuid:?}");
                        }

                        // update the storage after processing the response
                        self.update_storage().await;
                    }


                    else => {
                        log::info!("No more messages to process, checking for tasks to compose proofs.");
                        let tasks = self.task_queue.write().await;
                        log::info!("Current task queue size: {}", tasks.len());
                        // now that we have processed all tasks, update the storage
                        // to reflect the current state of the task queue.
                        self.update_storage().await;

                        // if any of the channels are closed, this indicates an error in one of the actors
                        // so we just exit the task manager.
                        if proof_rx.is_closed() {
                            log::info!("Proof composer channel is closed, exiting task manager.");
                            return Ok(());
                        }

                        if message_paid_events.is_closed() {
                            log::info!("No more message paid events to process, waiting for new events.");
                            break;
                        }



                    }
                }
            }
        }
    }
}

use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub uuid: Uuid,
    pub tx: TxHashWithSlot,
    pub state: TaskState,
}

impl Task {
    pub fn new(tx: TxHashWithSlot, state: TaskState) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            tx,
            state,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TaskState {
    ComposeProof,
    SubmitMessage { payload: Vec<u8> },
    Completed,
    Failed(String),
}
