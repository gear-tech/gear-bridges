use alloy_primitives::FixedBytes;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use primitive_types::H160;
use primitive_types::H256;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

use crate::message_relayer::common::{EthereumSlotNumber, TxHashWithSlot};
use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

use crate::message_relayer::eth_to_gear::paid_token_transfers::proof_composer::ProofComposerRequest;
use crate::message_relayer::eth_to_gear::paid_token_transfers::proof_composer::ProofComposerResponse;
use crate::message_relayer::eth_to_gear::paid_token_transfers::storage::Storage;

pub struct TaskManager {
    /// Queue of tasks to be processed given
    /// that their dependencies are met.
    pub task_queue: RwLock<BTreeMap<Uuid, Task>>,

    pub storage: Storage,
    pub api_provider: ApiProviderConnection,
    pub eth_api: EthApi,
    pub beacon_client: BeaconClient,

    pub bridging_payment_address: H160,
    pub checkpoint_light_client_address: H256,
    pub historical_proxy_client_address: H256,
    pub vft_manager_client_address: H256,
    pub suri: String,
}

impl TaskManager {
    pub fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        bridging_payment_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_client_address: H256,
        vft_manager_client_address: H256,
        suri: String,
        storage: Storage,
    ) -> Arc<Self> {
        Arc::new(Self {
            task_queue: RwLock::new(BTreeMap::new()),

            api_provider,
            eth_api,
            beacon_client,

            bridging_payment_address,
            checkpoint_light_client_address,
            historical_proxy_client_address,
            vft_manager_client_address,
            suri,

            storage,
        })
    }

    pub async fn update_storage(&self) {
        let tasks = self.task_queue.read().unwrap();
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

    pub fn enqueue(&self, task: Task) {
        let mut queue = self.task_queue.write().unwrap();
        queue.insert(task.uuid, task);
    }

    pub async fn run(
        self: Arc<Self>,
        (proof_tx, mut proof_rx): (
            UnboundedSender<ProofComposerRequest>,
            UnboundedReceiver<ProofComposerResponse>,
        ),
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
    ) -> anyhow::Result<()> {
        if let Ok(tasks) = self.read_storage().await {
            let mut queue = self.task_queue.write().unwrap();
            for (uuid, task) in tasks {
                queue.insert(uuid, task);
            }
        } else {
            log::warn!("Failed to read tasks from storage, starting with an empty task queue.");
        }

        loop {
            let mut tasks = self.task_queue.write().unwrap();

            loop {
                tokio::select! {
                    Some(tx) = message_paid_events.recv() => {
                        let task = Task::paid_event(tx.tx_hash, tx.slot_number);
                        self.enqueue(task);
                    }

                    Some(ProofComposerResponse { task_uuid, payload }) = proof_rx.recv() => {
                        log::info!("Received proof for task {task_uuid:?}: {payload:?}");
                    }



                    else => {
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
                            if let TaskState::PaidEvent { tx } = &task.task_state {
                                if let Err(_) = proof_tx.send(ProofComposerRequest {
                                    task_uuid: *task_uuid,
                                    tx: tx.clone(),
                                }) {
                                    // in case of an error, save the current state of the task queue
                                    self.update_storage().await;
                                    log::error!("Proof composer stopped accepting messages, exiting.");
                                    return Ok(());
                                }

                                *task = Task {
                                    task_state: TaskState::ComposeProof {
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

    pub task_state: TaskState,
}

impl Task {
    pub fn new(task_state: TaskState) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            task_state,
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
