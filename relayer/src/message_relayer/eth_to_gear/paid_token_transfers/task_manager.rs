use alloy_primitives::FixedBytes;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

use crate::message_relayer::common::{EthereumSlotNumber, TxHashWithSlot};
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

    pub async fn enqueue(&self, task: Task) {
        let mut queue = self.task_queue.write().await;
        queue.insert(task.uuid, task);
    }

    pub async fn run(
        self: Arc<Self>,
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

        loop {
            loop {
                tokio::select! {
                    Some(tx) = message_paid_events.recv() => {
                        let task = Task::paid_event(tx.tx_hash, tx.slot_number);
                        self.enqueue(task).await;
                    }

                    Some(ProofComposerResponse { task_uuid, payload }) = proof_rx.recv() => {
                        log::info!("Received proof for task {task_uuid:?}: {payload:?}");
                        let mut tasks = self.task_queue.write().await;
                        match tasks.get_mut(&task_uuid) {
                            Some(task) => {
                                if let TaskState::ComposeProof { tx } = &task.state {
                                    if let Err(err) = message_sender.send(Message {
                                        task_uuid,
                                        payload,
                                        tx_hash: tx.tx_hash,
                                    }) {
                                        log::error!("Failed to send message for task {task_uuid:?}: {err}");
                                        continue;
                                    }
                                    task.state = TaskState::PaidEvent { tx: tx.clone() };
                                    log::info!("Task {task_uuid:?} state updated to PaidEvent.");
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
                        if let Some(task) = tasks.remove(&task_uuid) {
                            match status {
                                SendStatus::Success => {
                                    self.completed.write().await.insert(task_uuid, task);
                                    log::info!("Task {task_uuid:?} completed successfully.");
                                }
                                SendStatus::Failure(err) => {
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
                        let mut tasks = self.task_queue.write().await;
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

                        for (task_uuid, task) in tasks.iter_mut() {
                            if let TaskState::PaidEvent { tx } = &task.state {
                                if proof_tx.send(ProofComposerRequest {
                                    task_uuid: *task_uuid,
                                    tx: tx.clone(),
                                }).is_err() {
                                    // in case of an error, save the current state of the task queue
                                    self.update_storage().await;
                                    log::error!("Proof composer stopped accepting messages, exiting.");
                                    return Ok(());
                                }

                                *task = Task {
                                    state: TaskState::ComposeProof {
                                        tx: tx.clone(),
                                    },
                                    ..task.clone()
                                };
                            }
                        }
                        // now that we have processed all tasks, update the storage
                        // to reflect the current state of the task queue.
                        self.update_storage().await;
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

    pub state: TaskState,
}

impl Task {
    pub fn new(state: TaskState) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            state,
        }
    }

    pub fn paid_event(tx_hash: FixedBytes<32>, slot_number: EthereumSlotNumber) -> Self {
        Self::new(TaskState::PaidEvent {
            tx: TxHashWithSlot {
                slot_number,
                tx_hash,
            },
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TaskState {
    /// Paid event was observed on Ethereum network.
    ///
    /// This task waits for the checkpoint light client on Vara to confirm the slot.
    PaidEvent {
        tx: TxHashWithSlot,
    },

    ComposeProof {
        tx: TxHashWithSlot,
    },
}
