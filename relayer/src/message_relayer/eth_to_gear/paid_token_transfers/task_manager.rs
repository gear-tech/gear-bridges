use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use futures::executor::block_on;
use gear_rpc_client::GearApi;
use primitive_types::H160;
use primitive_types::H256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::{broadcast::Receiver, mpsc::UnboundedReceiver};

use crate::message_relayer::common::ethereum::find_slot_by_block_number;
use crate::message_relayer::common::{
    gear::block_listener::GearBlock, EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot,
};
use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

pub struct TaskManager {
    /// Queue of tasks to be processed given
    /// that their dependencies are met.
    pub task_queue: RwLock<BTreeMap<Uuid, Task>>,

    pub running_tasks: RwLock<BTreeMap<Uuid, Task>>,
    pub failed_tasks: RwLock<BTreeMap<Uuid, (Task, anyhow::Error)>>,

    /// Completed tasks that have been processed.
    pub completed_tasks: RwLock<Vec<Task>>,

    pub api_provider: ApiProviderConnection,
    pub eth_api: EthApi,
    pub beacon_client: BeaconClient,

    /// Set of checkpoints that have been observer
    /// by the checkpoint light client.
    ///
    /// Once new entr is added into this set,
    /// we walk `task_queue` and check if `PaidEvent` tasks
    /// can be processed.
    pub checkpoints: RwLock<BTreeSet<EthereumSlotNumber>>,

    pub bridging_payment_address: H160,
    pub checkpoint_light_client_address: H256,
    pub historical_proxy_client_address: H256,
    pub vft_manager_client_address: H256,
    pub suri: String,
}

/// A context for tasks.
///
/// This type stores API clients and the task manager reference itself.
pub struct TaskContext {
    pub gear_api: Option<GearApi>,
    pub eth_api: Option<EthApi>,
    pub beacon_client: Option<BeaconClient>,
    pub task_manager: Arc<TaskManager>,
}

impl TaskContext {
    pub fn eth_api(&self) -> &EthApi {
        self.eth_api
            .as_ref()
            .expect("EthApi is required for this task")
    }

    pub fn gear_api(&self) -> &GearApi {
        self.gear_api
            .as_ref()
            .expect("GearApi is required for this task")
    }

    pub fn gclient_api(&self, suri: &str) -> anyhow::Result<gclient::GearApi> {
        gclient::GearApi::from(self.gear_api().api.clone())
            .with(suri)
            .map_err(|err| anyhow::anyhow!("Failed to set suri: {err}"))
    }

    pub fn beacon_client(&self) -> &BeaconClient {
        self.beacon_client
            .as_ref()
            .expect("BeaconClient is required for this task")
    }
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
    ) -> Arc<Self> {
        Arc::new(Self {
            task_queue: RwLock::new(BTreeMap::new()),
            running_tasks: RwLock::new(BTreeMap::new()),
            failed_tasks: RwLock::new(BTreeMap::new()),
            completed_tasks: RwLock::new(Vec::new()),
            api_provider,
            eth_api,
            beacon_client,
            checkpoints: RwLock::new(BTreeSet::new()),
            bridging_payment_address,
            checkpoint_light_client_address,
            historical_proxy_client_address,
            vft_manager_client_address,
            suri,
        })
    }

    pub fn add_checkpoint(&self, slot_number: EthereumSlotNumber) {
        self.checkpoints.write().unwrap().insert(slot_number);
        log::info!("Added checkpoint for slot {slot_number}");
    }

    pub fn enqueue(&self, task: Task) {
        let mut queue = self.task_queue.write().unwrap();
        queue.insert(task.uuid, task);
    }

    pub fn complete(&self, task: Uuid) {
        if let Some(task) = self.running_tasks.write().unwrap().remove(&task) {
            log::info!("Task {task:?} completed");
            self.completed_tasks.write().unwrap().push(task);
        } else {
            log::warn!("Attempted to complete a task that was not running: {task}");
        }
    }

    pub fn fail(&self, task: Uuid, error: anyhow::Error) {
        if let Some(task) = self.running_tasks.write().unwrap().remove(&task) {
            log::error!("Task {task:?} failed with error: {error}");
            self.failed_tasks
                .write()
                .unwrap()
                .insert(task.uuid, (task, error));
        } else {
            log::warn!("Attempted to fail a task that was not running: {task}");
        }
    }

    fn context_for(self: &Arc<Self>, task: &Task) -> TaskContext {
        TaskContext {
            gear_api: task.needs_gear_api().then(|| self.api_provider.client()),
            eth_api: task.needs_eth_api().then(|| self.eth_api.clone()),
            beacon_client: task.needs_eth_api().then(|| self.beacon_client.clone()),
            task_manager: Arc::clone(self),
        }
    }

    pub async fn run(
        self: Arc<Self>,
        mut checkpoints: UnboundedReceiver<EthereumSlotNumber>,
        mut message_paid_events: UnboundedReceiver<TxHashWithSlot>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                Some(slot_number) = checkpoints.recv() => {
                    self.add_checkpoint(slot_number);
                }

                Some(tx_hash) = message_paid_events.recv() => {
                    self.enqueue(Task::paid_event(tx_hash.tx_hash, tx_hash.slot_number));
                }

                else => {
                     let mut to_process = Vec::new();

                    let mut task_queue = self.task_queue.write().unwrap();
                    let mut failed = self.failed_tasks.write().unwrap();

                    for (uuid, (task, error)) in failed.iter() {
                        if task.retries < 3 {
                            log::warn!("Retrying task {task:?} due to error: {error}");
                            task_queue.insert(*uuid, Task {
                                retries: task.retries + 1,
                                ..task.clone()
                            });
                        } else {
                            log::error!("Task {task:?} failed after 3 retries: {error}");
                        }
                    }

                    for (uuid, task) in task_queue.iter() {
                        if self.dependencies_met(task) {
                            to_process.push(*uuid);
                        }
                    }

                    for uuid in to_process {
                        if let Some(task) = task_queue.remove(&uuid)
                            .or_else(|| failed.remove(&uuid).map(|(t, _)| t)) {
                            let ctx = self.context_for(&task);
                            let task_uuid = task.uuid;
                            self.running_tasks
                                .write()
                                .unwrap()
                                .insert(task.uuid, task.clone());
                            match task.task_kind {
                                TaskKind::PaidEvent { ref tx } => {
                                    let historical_proxy_address =
                                        self.historical_proxy_client_address;
                                    let suri = self.suri.clone();
                                    let tx = tx.clone();
                                    tokio::task::spawn_blocking(move || block_on(async move {
                                        let proof_composer = super::proof_composer::ProofComposerTask::new(
                                            &ctx,
                                            tx,
                                            historical_proxy_address,
                                            suri.clone(),
                                        );

                                        match proof_composer.run().await {
                                            Ok(()) => {
                                                ctx.task_manager.complete(task_uuid);
                                            }

                                            Err(err) => {
                                                ctx.task_manager.fail(task_uuid, err);
                                            }
                                        }
                                    }));
                                }

                                TaskKind::SubmitMessage { ref payload, ref tx } => {
                                    let vft_manager_client_address =
                                        self.vft_manager_client_address;
                                    let suri = self.suri.clone();
                                    let historical_proxy_address =
                                        self.historical_proxy_client_address;
                                    let tx = tx.clone();
                                    let payload = payload.clone();
                                    tokio::task::spawn_blocking(move || block_on(async move {
                                        let task = super::submit_message::SubmitMessageTask::new(
                                            &ctx,
                                            payload,
                                            tx,
                                            historical_proxy_address,
                                            vft_manager_client_address,
                                            suri,
                                        );

                                        match task.run().await {
                                            Ok(()) => {
                                                ctx.task_manager.complete(task_uuid);
                                            }

                                            Err(err) => {
                                                ctx.task_manager.fail(task_uuid, err);
                                            }
                                        }
                                    }));
                                }

                                _ => unimplemented!()
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn dependencies_met(&self, task: &Task) -> bool {
        match &task.task_kind {
            TaskKind::PaidEvent { tx } => {
                let checkpoints = self.checkpoints.read().unwrap();
                checkpoints
                    .last()
                    .map_or(false, |last_checkpoint| tx.slot_number <= *last_checkpoint)
            }

            _ => true,
        }
    }
}

use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Task {
    pub uuid: Uuid,
    pub retries: usize,
    pub task_kind: TaskKind,
}

impl Task {
    pub fn new(task_kind: TaskKind) -> Self {
        Self {
            uuid: Uuid::now_v7(),
            task_kind,
            retries: 0,
        }
    }

    pub fn paid_event(tx_hash: FixedBytes<32>, slot_number: EthereumSlotNumber) -> Self {
        Self::new(TaskKind::PaidEvent {
            tx: TxHashWithSlot {
                slot_number,
                tx_hash,
            },
        })
    }

    pub fn submit_message(payload: EthToVaraEvent, tx: TxHashWithSlot) -> Self {
        Self::new(TaskKind::SubmitMessage { payload, tx })
    }

    pub fn needs_gear_api(&self) -> bool {
        matches!(
            self.task_kind,
            TaskKind::SubmitMessage { .. } | TaskKind::PaidEvent { .. }
        )
    }

    pub fn needs_eth_api(&self) -> bool {
        matches!(
            self.task_kind,
            TaskKind::PaidEvent { .. } | TaskKind::ExtractMessagePaidEvents { .. }
        )
    }
}
#[derive(Clone, Debug)]
pub enum TaskKind {
    #[allow(dead_code)]
    // todo: shutdown task is not used yet
    Shutdown,

    ExtractCheckpoints {
        block: GearBlock,
    },

    ExtractMessagePaidEvents {
        block: EthereumBlockNumber,
        slot_number: EthereumSlotNumber,
    },

    /// Paid event was observed on Ethereum network.
    ///
    /// This task waits for the checkpoint light client on Vara to confirm the slot.
    PaidEvent {
        tx: TxHashWithSlot,
    },
    /// Submit a message to the Gear network.
    SubmitMessage {
        payload: EthToVaraEvent,
        tx: TxHashWithSlot,
    },
}
