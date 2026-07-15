use crate::{
    common::{sync_authority_set_id, SyncStepCount},
    message_relayer::common::GearBlock,
    proof_storage::ProofStorage,
    rpc,
};
use futures::{executor::block_on, FutureExt};
use gear_common::api_provider::ApiProviderConnection;
use prometheus::IntGauge;
use prover::proving::GenesisConfig;
use std::{
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{error::TryRecvError, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use utils_prometheus::{impl_metered_service, MeteredService};

pub struct AuthoritySetSyncIo {
    response: UnboundedReceiver<Response>,
    requests: UnboundedSender<Request>,
}

pub enum Request {
    ForceSync(Box<GearBlock>),
    Initialize,
}

pub enum Response {
    AuthoritySetSynced(u64, u32),
}

impl AuthoritySetSyncIo {
    pub fn new(response: UnboundedReceiver<Response>, requests: UnboundedSender<Request>) -> Self {
        Self { response, requests }
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.response.recv().await
    }

    pub fn send(&self, block: GearBlock) -> bool {
        self.requests
            .send(Request::ForceSync(Box::new(block)))
            .is_ok()
    }

    pub fn initialize(&self) -> bool {
        self.requests.send(Request::Initialize).is_ok()
    }
}

impl_metered_service!(
    struct Metrics {
        latest_proven_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_proven_era",
            "Latest proven era number",
        ),
        latest_observed_gear_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
    }
);

#[derive(Clone)]
struct SyncContext {
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
    count_thread: Option<usize>,
    metrics: Metrics,
}

struct SharedSyncRequest {
    relayer_id: String,
    priority: i64,
    sequence: u64,
    context: SyncContext,
    block: GearBlock,
    responses: UnboundedSender<Response>,
    reply: oneshot::Sender<anyhow::Result<(SyncStepCount, u64)>>,
}

/// Shared worker that serializes authority-set proving across relayers in one process.
pub struct SharedAuthoritySetSync {
    requests: UnboundedSender<SharedSyncRequest>,
}

impl SharedAuthoritySetSync {
    pub fn new() -> Arc<Self> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                // Supervise the shared worker: if `process_shared_sync_requests` returns
                // an error or panics, log and restart it with the same receiver instead
                // of letting the task die. A dead worker otherwise takes down authority
                // set sync for every relayer until the whole process is restarted.
                loop {
                    let outcome = AssertUnwindSafe(process_shared_sync_requests(&mut rx))
                        .catch_unwind()
                        .await;
                    match outcome {
                        Ok(Ok(())) => {
                            log::info!("Shared authority set sync worker exiting");
                            break;
                        }
                        Ok(Err(err)) => {
                            log::error!(
                                "Shared authority set sync worker failed: {err}; restarting worker"
                            );
                            continue;
                        }
                        Err(panic_payload) => {
                            log::error!(
                                "Shared authority set sync worker panicked: {}; restarting worker",
                                panic_message(&panic_payload)
                            );
                            continue;
                        }
                    }
                }
            })
        });

        Arc::new(Self { requests: tx })
    }

    fn create_handle(
        &self,
        relayer_id: String,
        priority: i64,
        context: SyncContext,
        responses: UnboundedSender<Response>,
    ) -> SharedAuthoritySetSyncHandle {
        SharedAuthoritySetSyncHandle {
            requests: self.requests.clone(),
            relayer_id,
            priority,
            sequence: Arc::new(AtomicU64::new(0)),
            context,
            responses,
        }
    }
}

struct SharedAuthoritySetSyncHandle {
    requests: UnboundedSender<SharedSyncRequest>,
    relayer_id: String,
    priority: i64,
    sequence: Arc<AtomicU64>,
    context: SyncContext,
    responses: UnboundedSender<Response>,
}

impl SharedAuthoritySetSyncHandle {
    async fn sync_authority_set(&self, block: &GearBlock) -> anyhow::Result<(SyncStepCount, u64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.requests
            .send(SharedSyncRequest {
                relayer_id: self.relayer_id.clone(),
                priority: self.priority,
                sequence,
                context: self.context.clone(),
                block: block.clone(),
                responses: self.responses.clone(),
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("shared authority set sync worker is not running"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("shared authority set sync worker dropped the request"))?
    }
}

async fn process_shared_sync_requests(
    requests: &mut UnboundedReceiver<SharedSyncRequest>,
) -> anyhow::Result<()> {
    const BATCH_SIZE: usize = 4096;
    let mut pending = Vec::with_capacity(BATCH_SIZE);

    loop {
        if pending.is_empty() {
            let Some(request) = requests.recv().await else {
                log::info!("Shared authority set sync request channel closed, exiting");
                return Ok(());
            };
            pending.push(request);
        }

        while pending.len() < BATCH_SIZE {
            match requests.try_recv() {
                Ok(request) => pending.push(request),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        let Some(index) = select_next_sync_request(&pending) else {
            continue;
        };
        let request = pending.remove(index);
        let relayer_id = request.relayer_id.clone();
        log::info!("Shared authority set sync selected relayer {relayer_id} for the next sync job");

        let mut context = request.context.clone();
        // Isolate each sync job from panics: a panic in `execute_sync_authority_set`
        // (e.g. from a stale/out-of-order block or a prover/storage panic) is converted
        // to an error reply so the requesting runner can recover, and the worker keeps
        // serving other relayers instead of being killed.
        let result = match AssertUnwindSafe(execute_sync_authority_set(
            &mut context.api_provider,
            &context.proof_storage,
            context.genesis_config,
            context.count_thread,
            &request.block,
            &request.responses,
            &context.metrics,
        ))
        .catch_unwind()
        .await
        {
            Ok(result) => result,
            Err(panic_payload) => {
                let msg = panic_message(&panic_payload);
                log::error!(
                    "Shared authority set sync job for relayer {relayer_id} panicked: {msg}"
                );
                Err(anyhow::anyhow!("authority set sync job panicked: {msg}"))
            }
        };

        let _ = request.reply.send(result);
    }
}

fn select_next_sync_request(pending: &[SharedSyncRequest]) -> Option<usize> {
    pending
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| right.sequence.cmp(&left.sequence))
        })
        .map(|(index, _)| index)
}

/// Best-effort extraction of a message from a `catch_unwind` panic payload.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    "unknown panic payload".to_string()
}

/// Authority set sync task which is responsible for synchronizing
/// authority set in proof-storage and generating new proofs.
///
/// Once proof is generated it is sent to merkle root relayer for further
/// processing.
pub struct AuthoritySetSync {
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,

    count_thread: Option<usize>,
    relayer_id: String,
    priority: i64,
    shared: Option<Arc<SharedAuthoritySetSync>>,

    metrics: Metrics,
}

impl MeteredService for AuthoritySetSync {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources().into_iter()
    }
}

impl AuthoritySetSync {
    pub async fn new(
        api_provider: ApiProviderConnection,
        proof_storage: Arc<dyn ProofStorage>,
        genesis_config: GenesisConfig,
        count_thread: Option<usize>,
        relayer_id: String,
        priority: i64,
        shared: Option<Arc<SharedAuthoritySetSync>>,
    ) -> Self {
        Self {
            api_provider,
            proof_storage,
            genesis_config,
            count_thread,
            relayer_id,
            priority,
            shared,
            metrics: Metrics::new(),
        }
    }

    pub fn run(self, mut blocks: Receiver<GearBlock>) -> AuthoritySetSyncIo {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = AuthoritySetSyncIo::new(rx, req_tx);

        if let Some(shared) = self.shared.clone() {
            let shared_handle = shared.create_handle(
                self.relayer_id.clone(),
                self.priority,
                SyncContext {
                    api_provider: self.api_provider.clone(),
                    proof_storage: self.proof_storage.clone(),
                    genesis_config: self.genesis_config,
                    count_thread: self.count_thread,
                    metrics: self.metrics.clone(),
                },
                tx.clone(),
            );

            let relayer_id = self.relayer_id.clone();
            tokio::spawn(async move {
                let mut runner = AuthoritySetSyncRunner {
                    relayer_id: relayer_id.clone(),
                    api_provider: self.api_provider,
                    proof_storage: self.proof_storage,
                    genesis_config: self.genesis_config,
                    count_thread: self.count_thread,
                    metrics: self.metrics,
                    shared_handle: Some(shared_handle),
                };

                loop {
                    if let Err(err) = runner.process(&mut blocks, &tx, &mut req_rx).await {
                        log::error!(
                            "Authority set sync for relayer {relayer_id} task failed: {err}"
                        );

                        match runner.api_provider.reconnect().await {
                            Ok(_) => {
                                log::info!(
                                    "Authority set sync for relayer {relayer_id}: reconnected to Gear API, resuming"
                                );
                                continue;
                            }
                            Err(err) => {
                                log::error!(
                                    "Authority set sync for relayer {relayer_id}: failed to reconnect to Gear API: {err}"
                                );
                                return;
                            }
                        }
                    } else {
                        log::info!("Authority set sync for relayer {relayer_id} task terminated");
                        break;
                    }
                }
            });
        } else {
            tokio::task::spawn_blocking(move || {
                block_on(async move {
                    let relayer_id = self.relayer_id.clone();
                    let mut runner = AuthoritySetSyncRunner {
                        relayer_id: relayer_id.clone(),
                        api_provider: self.api_provider,
                        proof_storage: self.proof_storage,
                        genesis_config: self.genesis_config,
                        count_thread: self.count_thread,
                        metrics: self.metrics,
                        shared_handle: None,
                    };

                    loop {
                        if let Err(err) = runner.process(&mut blocks, &tx, &mut req_rx).await {
                            log::error!(
                                "Authority set sync for relayer {relayer_id} task failed: {err}"
                            );

                            match runner.api_provider.reconnect().await {
                                Ok(_) => {
                                    log::info!(
                                        "Authority set sync for relayer {relayer_id}: reconnected to Gear API, resuming"
                                    );
                                    continue;
                                }
                                Err(err) => {
                                    log::error!(
                                        "Authority set sync for relayer {relayer_id}: failed to reconnect to Gear API: {err}"
                                    );
                                    return;
                                }
                            }
                        } else {
                            log::info!(
                                "Authority set sync for relayer {relayer_id} task terminated"
                            );
                            break;
                        }
                    }
                })
            });
        }

        io
    }
}

struct AuthoritySetSyncRunner {
    relayer_id: String,
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
    count_thread: Option<usize>,
    metrics: Metrics,
    shared_handle: Option<SharedAuthoritySetSyncHandle>,
}

impl AuthoritySetSyncRunner {
    async fn process(
        &mut self,
        blocks: &mut Receiver<GearBlock>,
        responses: &UnboundedSender<Response>,
        force_sync: &mut UnboundedReceiver<Request>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                req = force_sync.recv() => {
                    match req {
                        Some(Request::ForceSync(block)) => {
                            log::info!("Authority set sync for relayer {}: force syncing authority set for block #{}", self.relayer_id, block.number());
                            let Some(_) = self.sync_authority_set_completely(&block, blocks, responses).await? else {
                                return Ok(());
                            };
                        }
                        Some(Request::Initialize) => {
                            let latest_proven_authority_set_id =
                                self.proof_storage.get_latest_authority_set_id().await;
                            let block = if latest_proven_authority_set_id.is_none() {
                                log::info!(
                                    "Authority set sync for relayer {}: no authority set found in proof storage, syncing from genesis",
                                    self.relayer_id
                                );
                                let genesis_authority_set_id = self.genesis_config.authority_set_id;
                                rpc::retry_gear(
                                    &mut self.api_provider,
                                    "authority set sync initialize",
                                    move |client| async move {
                                        let genesis_block_hash = client
                                            .find_era_first_block(genesis_authority_set_id + 1)
                                            .await?;
                                        let genesis_block =
                                            client.get_block_at(genesis_block_hash).await?;
                                        GearBlock::from_subxt_block(&client, genesis_block).await
                                    },
                                )
                                .await?
                            } else {
                                log::info!(
                                    "Authority set sync for relayer {}: checking authority set catch-up on startup (latest proven {latest_proven_authority_set_id:?})",
                                    self.relayer_id
                                );
                                rpc::retry_gear(
                                    &mut self.api_provider,
                                    "authority set sync startup catch-up",
                                    move |client| async move {
                                        let block_hash = client.latest_finalized_block().await?;
                                        let block = client.get_block_at(block_hash).await?;
                                        GearBlock::from_subxt_block(&client, block).await
                                    },
                                )
                                .await?
                            };

                            let Some(_) = self
                                .sync_authority_set_completely(&block, blocks, responses)
                                .await?
                            else {
                                return Ok(());
                            };
                        }
                        None => {
                            log::warn!(
                                "Authority set sync for relayer {}: force sync channel closed, exiting",
                                self.relayer_id
                            );
                            return Ok(());
                        }
                    }
                }

                block = blocks.recv() => {
                    match block {
                        Ok(block) => {
                            if !super::storage::authority_set_changed(&block) {
                                continue;
                            }

                            let Some(_) = self.sync_authority_set_completely(&block, blocks, responses).await? else {
                                return Ok(());
                            };
                        }

                        Err(RecvError::Lagged(n)) => {
                            log::error!(
                                "Authority set sync for relayer {}: Gear block listener lagged behind {n} blocks, skipping some blocks",
                                self.relayer_id
                            );
                            continue;
                        }

                        Err(RecvError::Closed) => {
                            log::warn!(
                                "Authority set sync for relayer {}: Gear block listener connection closed, exiting",
                                self.relayer_id
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    async fn sync_authority_set_completely(
        &mut self,
        initial_block: &GearBlock,
        blocks: &mut Receiver<GearBlock>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<Option<u64>> {
        let (sync_steps, authority_set_id) =
            self.sync_authority_set(initial_block, responses).await?;
        if sync_steps == 0 {
            log::info!(
                "Authority set sync for relayer {}: authority set #{authority_set_id} is already in sync at block #{}",
                self.relayer_id,
                initial_block.number()
            );
            // Fall through to emit `Response::AuthoritySetSynced` so the merkle root
            // relayer drains `waiting_for_authority_set_sync[id]` and any parked HTTP
            // requests even when the set was already caught up (e.g. a `ForceSync` /
            // `Initialize` that races with an already-completed sync). The receiver
            // tolerates "no blocks to sync for authority set #{id}".
        } else {
            log::info!(
                "Authority set sync for relayer {}: syncing authority set #{authority_set_id}",
                self.relayer_id
            );
            loop {
                let (sync_steps, _) = match blocks.recv().await {
                    Ok(block) => self.sync_authority_set(&block, responses).await?,

                    Err(RecvError::Closed) => {
                        log::warn!(
                            "Authority set sync for relayer {}: Gear block listener connection closed",
                            self.relayer_id
                        );
                        return Ok(None);
                    }

                    Err(RecvError::Lagged(n)) => {
                        log::error!(
                            "Authority set sync for relayer {}: Gear block listener lagged behind {n} blocks, skipping some blocks",
                            self.relayer_id
                        );
                        continue;
                    }
                };

                if sync_steps == 0 {
                    break;
                } else {
                    log::info!(
                        "Authority set sync for relayer {}: synced {sync_steps} authority sets",
                        self.relayer_id
                    );
                }
            }

            log::info!(
                "Authority set sync for relayer {}: authority set #{authority_set_id} is in sync",
                self.relayer_id
            );
        }

        if responses
            .send(Response::AuthoritySetSynced(
                authority_set_id,
                initial_block.number(),
            ))
            .is_err()
        {
            return Ok(None);
        }

        Ok(Some(authority_set_id))
    }

    async fn sync_authority_set(
        &mut self,
        block: &GearBlock,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<(SyncStepCount, u64)> {
        if let Some(handle) = &self.shared_handle {
            return handle.sync_authority_set(block).await;
        }

        execute_sync_authority_set(
            &mut self.api_provider,
            &self.proof_storage,
            self.genesis_config,
            self.count_thread,
            block,
            responses,
            &self.metrics,
        )
        .await
    }
}

async fn execute_sync_authority_set(
    api_provider: &mut ApiProviderConnection,
    proof_storage: &Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
    count_thread: Option<usize>,
    block: &GearBlock,
    responses: &UnboundedSender<Response>,
    metrics: &Metrics,
) -> anyhow::Result<(SyncStepCount, u64)> {
    let finalized_head = block.hash();
    let proof_storage = proof_storage.clone();
    let responses = responses.clone();

    let (sync_steps, latest_authority_set_id, latest_proven_authority_set_id) =
        rpc::retry_gear(api_provider, "authority set sync", move |gear_api| {
            let proof_storage = proof_storage.clone();
            let responses = responses.clone();
            async move {
                let latest_authority_set_id = gear_api.authority_set_id(finalized_head).await?;
                let latest_proven_authority_set_id =
                    proof_storage.get_latest_authority_set_id().await;
                let sync_steps = sync_authority_set_id(
                    &gear_api,
                    &proof_storage,
                    genesis_config,
                    latest_authority_set_id,
                    latest_proven_authority_set_id,
                    &responses,
                    count_thread,
                )
                .await?;
                Ok::<_, anyhow::Error>((
                    sync_steps,
                    latest_authority_set_id,
                    latest_proven_authority_set_id,
                ))
            }
        })
        .await?;

    metrics
        .latest_observed_gear_era
        .set(latest_authority_set_id as i64);
    if let Some(latest_proven) = latest_proven_authority_set_id {
        metrics.latest_proven_era.set(latest_proven as i64);
    }

    Ok((sync_steps, latest_authority_set_id))
}
