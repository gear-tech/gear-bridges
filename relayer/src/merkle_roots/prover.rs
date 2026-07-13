use crate::{
    prover_interface::{self, FinalProof},
    rpc,
};
use futures::executor::block_on;
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::dto::RawBlockInclusionProof;
use primitive_types::H256;
use prometheus::{IntCounter, IntGauge, IntGaugeVec, Opts};
use prover::proving::{GenesisConfig, ProofWithCircuitData};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{error::TryRecvError, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

#[derive(Clone)]
pub struct Request {
    pub block_number: u32,
    pub block_hash: H256,
    pub merkle_root: H256,
    pub queue_id: u64,
    pub inner_proof: ProofWithCircuitData,
    pub batch: bool,
    pub block_inclusion_proof: RawBlockInclusionProof,
}

pub enum Response {
    /// A single finality proof for a block.
    Single {
        block_number: u32,
        merkle_root: H256,
        proof: FinalProof,
    },

    /// Finality proof for a batch of blocks. Contains
    /// the latest block number and its merkle root, plus all the
    /// blocks in the batch for whom the proof is valid.
    Batched {
        block_number: u32,
        merkle_root: H256,
        proof: FinalProof,

        batch_roots: Vec<(u32, H256)>,
    },
}

#[derive(Clone)]
struct ProverContext {
    api_provider: ApiProviderConnection,
    genesis_config: GenesisConfig,
    count_thread: Option<usize>,
    gnark_data_path: PathBuf,
}

enum RequestSender {
    Direct(UnboundedSender<Request>),
    Shared {
        relayer_id: String,
        priority: i64,
        context: ProverContext,
        requests: UnboundedSender<SharedRequest>,
        responses: UnboundedSender<Response>,
        sequence: Arc<AtomicU64>,
    },
}

pub struct FinalityProverIo {
    requests: RequestSender,
    responses: UnboundedReceiver<Response>,
}

impl FinalityProverIo {
    pub fn new_direct(
        requests: UnboundedSender<Request>,
        responses: UnboundedReceiver<Response>,
    ) -> Self {
        Self {
            requests: RequestSender::Direct(requests),
            responses,
        }
    }

    fn new_shared(
        relayer_id: String,
        priority: i64,
        context: ProverContext,
        requests: UnboundedSender<SharedRequest>,
        responses: UnboundedReceiver<Response>,
        response_tx: UnboundedSender<Response>,
        sequence: Arc<AtomicU64>,
    ) -> Self {
        Self {
            requests: RequestSender::Shared {
                relayer_id,
                priority,
                context,
                requests,
                responses: response_tx,
                sequence,
            },
            responses,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prove(
        &mut self,
        block_number: u32,
        block_hash: H256,
        merkle_root: H256,
        inner_proof: ProofWithCircuitData,
        queue_id: u64,
        batch: bool,
        block_inclusion_proof: RawBlockInclusionProof,
    ) -> bool {
        let request = Request {
            block_number,
            block_hash,
            merkle_root,
            inner_proof,
            queue_id,
            batch,
            block_inclusion_proof,
        };

        match &self.requests {
            RequestSender::Direct(requests) => requests.send(request).is_ok(),
            RequestSender::Shared {
                relayer_id,
                priority,
                context,
                requests,
                responses,
                sequence,
            } => requests
                .send(SharedRequest {
                    relayer_id: relayer_id.clone(),
                    priority: *priority,
                    sequence: sequence.fetch_add(1, Ordering::Relaxed),
                    context: context.clone(),
                    request,
                    responses: responses.clone(),
                })
                .is_ok(),
        }
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

impl_metered_service!(
    struct Metrics {
        pending_requests: IntGauge = IntGauge::new(
            "merkle_root_relayer_pending_requests",
            "Number of pending merkle root requests"
        ),
        currently_processing: IntGauge = IntGauge::new(
            "merkle_root_relayer_currently_processing",
            "Number of merkle root requests currently being processed"
        ),
        current_root_block: IntGauge = IntGauge::new(
            "merkle_root_relayer_current_root_block",
            "Block number of the current merkle root being processed"
        ),
        pending_requests_by_relayer: IntGaugeVec = IntGaugeVec::new(
            Opts::new(
                "merkle_root_relayer_pending_requests_by_relayer",
                "Number of pending shared prover requests by relayer"
            ),
            &["relayer"],
        ),
        currently_processing_by_relayer: IntGaugeVec = IntGaugeVec::new(
            Opts::new(
                "merkle_root_relayer_currently_processing_by_relayer",
                "Number of shared prover requests currently being processed by relayer"
            ),
            &["relayer"],
        ),
        current_relayer_priority: IntGaugeVec = IntGaugeVec::new(
            Opts::new(
                "merkle_root_relayer_current_relayer_priority",
                "Priority of the relayer currently selected by the shared prover"
            ),
            &["relayer"],
        ),
        current_root_block_by_relayer: IntGaugeVec = IntGaugeVec::new(
            Opts::new(
                "merkle_root_relayer_current_root_block_by_relayer",
                "Block number currently being processed by the shared prover for each relayer"
            ),
            &["relayer"],
        ),
        dropped_responses: IntCounter = IntCounter::new(
            "merkle_root_relayer_dropped_prover_responses",
            "Number of shared prover responses dropped because the receiving relayer channel was closed"
        ),
        failed_requests: IntCounter = IntCounter::new(
            "merkle_root_relayer_failed_prover_requests",
            "Number of shared prover requests that failed during proof generation"
        ),
        max_proof_time: IntGauge = IntGauge::new(
            "merkle_root_relayer_max_proof_time_seconds",
            "Maximum time taken to generate a proof (in seconds)"
        ),
        last_proof_time: IntGauge = IntGauge::new(
            "merkle_root_relayer_last_proof_time_seconds",
            "Time taken to generate the last proof (in seconds)"
        ),
        min_proof_time: IntGauge = IntGauge::new(
            "merkle_root_relayer_min_proof_time_seconds",
            "Minimum time taken to generate a proof (in seconds)"
        ),
    }
);

/// A separate thread responsible for running block finality prover.
pub struct FinalityProver {
    context: ProverContext,

    metrics: Metrics,
}

impl MeteredService for FinalityProver {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl FinalityProver {
    pub fn new(
        api_provider: ApiProviderConnection,
        genesis_config: GenesisConfig,
        count_thread: Option<usize>,
        gnark_data_path: PathBuf,
    ) -> Self {
        Self {
            context: ProverContext {
                api_provider,
                genesis_config,
                count_thread,
                gnark_data_path,
            },

            metrics: Metrics::new(),
        }
    }

    pub fn run(mut self) -> FinalityProverIo {
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (res_tx, res_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = FinalityProverIo::new_direct(req_tx, res_rx);

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                if let Err(e) = self.process(&mut req_rx, &res_tx).await {
                    log::error!("Error processing finality prover requests: {e}");
                } else {
                    log::info!("Prover exiting");
                }
            })
        });

        io
    }

    async fn process(
        &mut self,
        requests: &mut UnboundedReceiver<Request>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        // Batch size which is equal approximately to 2 full queues.
        const BATCH_SIZE: usize = 4096;

        let mut batch_vec = Vec::with_capacity(BATCH_SIZE);
        // Group requests by authority set ID and queue ID, storing all blocks for each set.
        // Use BTreeMap to have a deterministic order of processing: from older to newer authority sets.
        let mut request_groups: BTreeMap<(u64, u64), BatchProofRequest> = BTreeMap::new();

        let mut non_batch_requests: Vec<Request> = Vec::new();

        loop {
            // Receives messages into `batch_vec` but has one big downside requiring another `recv_many`
            // down below: will receive only one element first *and* only then will receive the full batch.
            let n = requests.recv_many(&mut batch_vec, BATCH_SIZE).await;
            if n == 0 {
                log::info!("Requests channel closed, exiting");
                break;
            }

            let n = if n == 1 {
                // attempt to receive more requests in 10 second timeout.
                let rest = tokio::time::timeout(
                    Duration::from_secs(10),
                    requests.recv_many(&mut batch_vec, BATCH_SIZE - 1),
                )
                .await;

                match rest {
                    Err(_) => {
                        log::info!("Only one request received, processing it immediately");
                        let request = batch_vec.pop().expect("at least one request is received");

                        self.metrics.pending_requests.set(0);
                        self.metrics.currently_processing.set(1);
                        self.metrics
                            .current_root_block
                            .set(request.block_number as i64);

                        let proof = self
                            .generate_proof(
                                request.block_number,
                                request.block_hash,
                                request.merkle_root,
                                request.inner_proof,
                                request.block_inclusion_proof,
                            )
                            .await?;

                        if responses
                            .send(Response::Single {
                                block_number: request.block_number,
                                merkle_root: request.merkle_root,
                                proof,
                            })
                            .is_err()
                        {
                            log::warn!("Response channel closed, exiting");
                            return Ok(());
                        }

                        continue;
                    }

                    // received more requests, process them in batch
                    Ok(rest) => rest + 1,
                }
            } else {
                n
            };

            log::info!("Received {n} requests, grouping by authority set...");

            for request in batch_vec.drain(..) {
                if !request.batch {
                    non_batch_requests.push(request);
                    continue;
                }

                let block_hash = request.block_hash;
                let authority_set_id = rpc::retry_gear(
                    &mut self.context.api_provider,
                    "prover authority set id",
                    move |gear_api| async move { gear_api.authority_set_id(block_hash).await },
                )
                .await?;
                match request_groups.entry((authority_set_id, request.queue_id)) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().add_request(request);
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(BatchProofRequest::new(request));
                    }
                }
            }

            self.metrics
                .pending_requests
                .set((non_batch_requests.len() + request_groups.len()) as i64);

            // sort by block number ascending to process older requests first
            non_batch_requests.sort_by_key(|r| r.block_number);

            // First process all non-batched requests, then batched requests.
            // Non batched requests are processed first as they're the most important ones, and
            // can only be created in two scenarios:
            // - kill-switch relayer requested proof
            // - relayer restarted and needs to catch up
            // - authority set changed and there are some old requests left
            for request in non_batch_requests.drain(..) {
                log::info!(
                    "Proving finality for block #{block_number} with merkle-root {merkle_root} (non-batched)",
                    block_number = request.block_number,
                    merkle_root = request.merkle_root,
                );
                self.metrics.currently_processing.set(1);
                self.metrics
                    .current_root_block
                    .set(request.block_number as i64);
                self.metrics
                    .pending_requests
                    .set(self.metrics.pending_requests.get() - 1);
                let proof = self
                    .generate_proof(
                        request.block_number,
                        request.block_hash,
                        request.merkle_root,
                        request.inner_proof,
                        request.block_inclusion_proof,
                    )
                    .await?;
                self.metrics.currently_processing.set(0);
                self.metrics.current_root_block.set(0);

                if responses
                    .send(Response::Single {
                        block_number: request.block_number,
                        merkle_root: request.merkle_root,
                        proof,
                    })
                    .is_err()
                {
                    log::warn!("Response channel closed, exiting");
                    return Ok(());
                }
            }

            for ((authority_set_id, queue_id), request) in request_groups.iter() {
                let BatchProofRequest {
                    block_number,
                    block_hash,
                    merkle_root,
                    inner_proof,
                    batch_roots,
                    block_inclusion_proof,
                } = request.clone();
                log::info!(
                    "Proving finality for latest block #{block_number} with authority set #{authority_set_id}, merkle-root {merkle_root} and queue #{queue_id} (will apply to {} blocks)",
                    batch_roots.len()
                );
                self.metrics
                    .currently_processing
                    .set(batch_roots.len() as i64 + 1);
                self.metrics.current_root_block.set(block_number as i64);
                self.metrics
                    .pending_requests
                    .set(self.metrics.pending_requests.get() - 1);

                let proof = self
                    .generate_proof(
                        block_number,
                        block_hash,
                        merkle_root,
                        inner_proof,
                        block_inclusion_proof,
                    )
                    .await?;

                if responses
                    .send(Response::Batched {
                        block_number,
                        merkle_root,
                        proof,
                        batch_roots,
                    })
                    .is_err()
                {
                    log::warn!("Response channel closed, exiting");
                    return Ok(());
                }
            }

            request_groups.clear();
        }

        Ok(())
    }

    async fn generate_proof(
        &mut self,
        block_number: u32,
        block_hash: H256,
        merkle_root: H256,
        inner_proof: ProofWithCircuitData,
        block_inclusion_proof: RawBlockInclusionProof,
    ) -> anyhow::Result<FinalProof> {
        generate_proof(
            &self.metrics,
            &mut self.context,
            block_number,
            block_hash,
            merkle_root,
            inner_proof,
            block_inclusion_proof,
        )
        .await
    }
}

async fn generate_proof(
    metrics: &Metrics,
    context: &mut ProverContext,
    block_number: u32,
    block_hash: H256,
    merkle_root: H256,
    inner_proof: ProofWithCircuitData,
    block_inclusion_proof: RawBlockInclusionProof,
) -> anyhow::Result<FinalProof> {
    log::info!("Generating merkle root proof for block #{block_number}");

    log::info!("Proving merkle root({merkle_root}) presence in block #{block_number}");

    let start = Instant::now();
    let genesis_config = context.genesis_config;
    let count_thread = context.count_thread;
    let gnark_data_path = context.gnark_data_path.clone();
    let proof = rpc::retry_gear(
        &mut context.api_provider,
        "prover finality proof",
        move |gear_api| {
            let inner_proof = inner_proof.clone();
            let block_inclusion_proof = block_inclusion_proof.clone();
            let gnark_data_path = gnark_data_path.clone();
            async move {
                prover_interface::prove_final(
                    &gear_api,
                    inner_proof,
                    genesis_config,
                    block_hash,
                    count_thread,
                    gnark_data_path,
                    Some(block_inclusion_proof),
                )
                .await
            }
        },
    )
    .await?;
    let elapsed = start.elapsed().as_secs_f64();
    log::info!("Proof for {merkle_root} generated (block #{block_number}) in {elapsed:.3} seconds",);

    metrics.last_proof_time.set(elapsed.ceil() as i64);
    if metrics.min_proof_time.get() == 0 || elapsed < metrics.min_proof_time.get() as f64 {
        metrics.min_proof_time.set(elapsed.ceil() as i64);
    }
    if elapsed > metrics.max_proof_time.get() as f64 {
        metrics.max_proof_time.set(elapsed.ceil() as i64);
    }

    Ok(proof)
}

struct SharedRequest {
    relayer_id: String,
    priority: i64,
    sequence: u64,
    context: ProverContext,
    request: Request,
    responses: UnboundedSender<Response>,
}

pub struct SharedFinalityProver {
    requests: UnboundedSender<SharedRequest>,
    sequence: Arc<AtomicU64>,
    metrics: Metrics,
}

impl MeteredService for SharedFinalityProver {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl SharedFinalityProver {
    pub fn new() -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let metrics = Metrics::new();
        let worker_metrics = metrics.clone();

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                if let Err(err) = process_shared_requests(&mut rx, &worker_metrics).await {
                    log::error!("Error processing shared finality prover requests: {err}");
                } else {
                    log::info!("Shared prover exiting");
                }
            })
        });

        Self {
            requests: tx,
            sequence: Arc::new(AtomicU64::new(0)),
            metrics,
        }
    }

    pub fn register(
        &self,
        relayer_id: String,
        priority: i64,
        api_provider: ApiProviderConnection,
        genesis_config: GenesisConfig,
        count_thread: Option<usize>,
        gnark_data_path: PathBuf,
    ) -> FinalityProverIo {
        let (response_tx, response_rx) = tokio::sync::mpsc::unbounded_channel();
        FinalityProverIo::new_shared(
            relayer_id,
            priority,
            ProverContext {
                api_provider,
                genesis_config,
                count_thread,
                gnark_data_path,
            },
            self.requests.clone(),
            response_rx,
            response_tx,
            self.sequence.clone(),
        )
    }
}

impl Default for SharedFinalityProver {
    fn default() -> Self {
        Self::new()
    }
}

async fn process_shared_requests(
    requests: &mut UnboundedReceiver<SharedRequest>,
    metrics: &Metrics,
) -> anyhow::Result<()> {
    const BATCH_SIZE: usize = 4096;

    let mut pending = Vec::with_capacity(BATCH_SIZE);

    loop {
        if pending.is_empty() {
            let Some(request) = requests.recv().await else {
                log::info!("Shared prover request channel closed, exiting");
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

        record_pending_requests(metrics, &pending);

        let Some(work) = take_next_shared_proof_work(&mut pending).await? else {
            continue;
        };

        let relayer_id = work.relayer_id().to_string();
        log::info!("Shared prover selected relayer {relayer_id} for the next proof job");
        match work {
            SelectedSharedProofWork::Single(request) => {
                process_shared_single_request(request, &relayer_id, metrics).await?;
            }
            SelectedSharedProofWork::Batch {
                authority_set_id,
                queue_id,
                latest,
                batch_roots,
            } => {
                process_shared_batch_request(
                    latest,
                    batch_roots,
                    authority_set_id,
                    queue_id,
                    &relayer_id,
                    metrics,
                )
                .await?;
            }
        }
    }
}

fn record_pending_requests(metrics: &Metrics, pending: &[SharedRequest]) {
    let mut counts = BTreeMap::<&str, usize>::new();
    for request in pending {
        *counts.entry(request.relayer_id.as_str()).or_default() += 1;
    }
    record_pending_request_counts(metrics, counts);
}

fn record_pending_request_counts<'a>(
    metrics: &Metrics,
    counts: impl IntoIterator<Item = (&'a str, usize)>,
) {
    let mut total = 0;
    metrics.pending_requests_by_relayer.reset();
    for (relayer_id, count) in counts {
        total += count;
        metrics
            .pending_requests_by_relayer
            .with_label_values(&[relayer_id])
            .set(count as i64);
    }
    metrics.pending_requests.set(total as i64);
}

fn record_current_shared_request(
    metrics: &Metrics,
    relayer_id: &str,
    priority: i64,
    block_number: u32,
    currently_processing: usize,
) {
    metrics
        .currently_processing
        .set(currently_processing as i64);
    metrics.current_root_block.set(block_number as i64);
    metrics.currently_processing_by_relayer.reset();
    metrics.current_relayer_priority.reset();
    metrics.current_root_block_by_relayer.reset();
    metrics
        .currently_processing_by_relayer
        .with_label_values(&[relayer_id])
        .set(currently_processing as i64);
    metrics
        .current_relayer_priority
        .with_label_values(&[relayer_id])
        .set(priority);
    metrics
        .current_root_block_by_relayer
        .with_label_values(&[relayer_id])
        .set(block_number as i64);
}

fn clear_current_shared_request(metrics: &Metrics) {
    metrics.currently_processing.set(0);
    metrics.current_root_block.set(0);
    metrics.currently_processing_by_relayer.reset();
    metrics.current_relayer_priority.reset();
    metrics.current_root_block_by_relayer.reset();
}

enum SelectedSharedProofWork {
    Single(SharedRequest),
    Batch {
        authority_set_id: u64,
        queue_id: u64,
        latest: SharedRequest,
        batch_roots: Vec<(u32, H256)>,
    },
}

impl SelectedSharedProofWork {
    fn relayer_id(&self) -> &str {
        match self {
            SelectedSharedProofWork::Single(request) => &request.relayer_id,
            SelectedSharedProofWork::Batch { latest, .. } => &latest.relayer_id,
        }
    }
}

async fn take_next_shared_proof_work(
    pending: &mut Vec<SharedRequest>,
) -> anyhow::Result<Option<SelectedSharedProofWork>> {
    let Some(relayer_id) = select_next_relayer_id(pending.iter().map(|request| {
        (
            request.relayer_id.as_str(),
            request.priority,
            request.sequence,
        )
    })) else {
        return Ok(None);
    };

    if let Some((index, _)) = pending
        .iter()
        .enumerate()
        .filter(|(_, request)| request.relayer_id == relayer_id && !request.request.batch)
        .min_by_key(|(_, request)| (request.request.block_number, request.sequence))
    {
        return Ok(Some(SelectedSharedProofWork::Single(pending.remove(index))));
    }

    let mut selected_relayer_requests = Vec::new();
    let mut index = 0;
    while index < pending.len() {
        if pending[index].relayer_id == relayer_id {
            selected_relayer_requests.push(pending.remove(index));
        } else {
            index += 1;
        }
    }

    let mut request_groups = BTreeMap::<(u64, u64), Vec<SharedRequestWithAuthoritySet>>::new();
    for mut request in selected_relayer_requests {
        let block_hash = request.request.block_hash;
        let authority_set_id = rpc::retry_gear(
            &mut request.context.api_provider,
            "shared prover authority set id",
            move |gear_api| async move { gear_api.authority_set_id(block_hash).await },
        )
        .await?;
        let queue_id = request.request.queue_id;
        request_groups
            .entry((authority_set_id, queue_id))
            .or_default()
            .push(SharedRequestWithAuthoritySet {
                request,
                authority_set_id: Some(authority_set_id),
            });
    }

    let Some(((authority_set_id, queue_id), selected_group)) = request_groups.pop_first() else {
        return Ok(None);
    };

    for (_, group) in request_groups {
        pending.extend(group.into_iter().map(|request| request.request));
    }

    let mut work = plan_shared_relayer_requests(selected_group);
    match work.pop() {
        Some(SharedProofWork::Batch {
            authority_set_id: work_authority_set_id,
            queue_id: work_queue_id,
            latest,
            batch_roots,
        }) => {
            debug_assert_eq!(authority_set_id, work_authority_set_id);
            debug_assert_eq!(queue_id, work_queue_id);
            Ok(Some(SelectedSharedProofWork::Batch {
                authority_set_id,
                queue_id,
                latest: latest.request,
                batch_roots,
            }))
        }
        Some(SharedProofWork::Single(request)) => {
            Ok(Some(SelectedSharedProofWork::Single(request.request)))
        }
        None => Ok(None),
    }
}

fn select_next_relayer_id<'a>(
    pending: impl IntoIterator<Item = (&'a str, i64, u64)>,
) -> Option<String> {
    pending
        .into_iter()
        .max_by(
            |(_, left_priority, left_sequence), (_, right_priority, right_sequence)| {
                left_priority
                    .cmp(right_priority)
                    .then_with(|| right_sequence.cmp(left_sequence))
            },
        )
        .map(|(relayer_id, _, _)| relayer_id.to_string())
}

async fn process_shared_single_request(
    request: SharedRequest,
    relayer_id: &str,
    metrics: &Metrics,
) -> anyhow::Result<()> {
    let priority = request.priority;
    let Request {
        block_number,
        block_hash,
        merkle_root,
        inner_proof,
        block_inclusion_proof,
        ..
    } = request.request;
    log::info!(
        "Shared prover proving relayer {relayer_id} block #{block_number} with merkle-root {merkle_root} (non-batched)",
    );
    record_current_shared_request(metrics, relayer_id, priority, block_number, 1);
    metrics
        .pending_requests
        .set((metrics.pending_requests.get() - 1).max(0));

    let mut context = request.context;
    let proof = match generate_proof(
        metrics,
        &mut context,
        block_number,
        block_hash,
        merkle_root,
        inner_proof,
        block_inclusion_proof,
    )
    .await
    {
        Ok(proof) => proof,
        Err(err) => {
            metrics.failed_requests.inc();
            clear_current_shared_request(metrics);
            return Err(err);
        }
    };

    clear_current_shared_request(metrics);

    send_shared_response(
        &request.responses,
        Response::Single {
            block_number,
            merkle_root,
            proof,
        },
        metrics,
        relayer_id,
    );

    Ok(())
}

async fn process_shared_batch_request(
    request: SharedRequest,
    batch_roots: Vec<(u32, H256)>,
    authority_set_id: u64,
    queue_id: u64,
    relayer_id: &str,
    metrics: &Metrics,
) -> anyhow::Result<()> {
    let priority = request.priority;
    let responses = request.responses;
    let Request {
        block_number,
        block_hash,
        merkle_root,
        inner_proof,
        block_inclusion_proof,
        ..
    } = request.request;
    log::info!(
        "Shared prover proving relayer {relayer_id} latest block #{block_number} with authority set #{authority_set_id}, merkle-root {merkle_root} and queue #{queue_id} (will apply to {} blocks)",
        batch_roots.len()
    );
    let currently_processing = batch_roots.len() + 1;
    record_current_shared_request(
        metrics,
        relayer_id,
        priority,
        block_number,
        currently_processing,
    );
    metrics
        .pending_requests
        .set((metrics.pending_requests.get() - currently_processing as i64).max(0));

    let mut context = request.context;
    let proof = match generate_proof(
        metrics,
        &mut context,
        block_number,
        block_hash,
        merkle_root,
        inner_proof,
        block_inclusion_proof,
    )
    .await
    {
        Ok(proof) => proof,
        Err(err) => {
            metrics.failed_requests.inc();
            clear_current_shared_request(metrics);
            return Err(err);
        }
    };

    clear_current_shared_request(metrics);

    send_shared_response(
        &responses,
        Response::Batched {
            block_number,
            merkle_root,
            proof,
            batch_roots,
        },
        metrics,
        relayer_id,
    );

    Ok(())
}

fn send_shared_response(
    responses: &UnboundedSender<Response>,
    response: Response,
    metrics: &Metrics,
    relayer_id: &str,
) {
    if responses.send(response).is_err() {
        metrics.dropped_responses.inc();
        log::warn!("Shared prover response channel for relayer {relayer_id} closed");
    }
}

struct SharedRequestWithAuthoritySet {
    request: SharedRequest,
    authority_set_id: Option<u64>,
}

trait SharedProofRequestInfo {
    fn is_batch(&self) -> bool;
    fn block_number(&self) -> u32;
    fn merkle_root(&self) -> H256;
    fn queue_id(&self) -> u64;
    fn authority_set_id(&self) -> Option<u64>;
}

impl SharedProofRequestInfo for SharedRequestWithAuthoritySet {
    fn is_batch(&self) -> bool {
        self.request.request.batch
    }

    fn block_number(&self) -> u32 {
        self.request.request.block_number
    }

    fn merkle_root(&self) -> H256 {
        self.request.request.merkle_root
    }

    fn queue_id(&self) -> u64 {
        self.request.request.queue_id
    }

    fn authority_set_id(&self) -> Option<u64> {
        self.authority_set_id
    }
}

enum SharedProofWork<T> {
    Single(T),
    Batch {
        authority_set_id: u64,
        queue_id: u64,
        latest: T,
        batch_roots: Vec<(u32, H256)>,
    },
}

fn plan_shared_relayer_requests<T: SharedProofRequestInfo>(
    requests: Vec<T>,
) -> Vec<SharedProofWork<T>> {
    let mut non_batch_requests = Vec::new();
    let mut request_groups: BTreeMap<(u64, u64), SharedBatchProofWork<T>> = BTreeMap::new();

    for request in requests {
        if !request.is_batch() {
            non_batch_requests.push(request);
            continue;
        }

        let authority_set_id = request
            .authority_set_id()
            .expect("batch requests must have authority set id");
        let queue_id = request.queue_id();
        match request_groups.entry((authority_set_id, queue_id)) {
            Entry::Occupied(mut entry) => entry.get_mut().add_request(request),
            Entry::Vacant(entry) => {
                entry.insert(SharedBatchProofWork::new(request));
            }
        }
    }

    non_batch_requests.sort_by_key(|request| request.block_number());

    let mut work = non_batch_requests
        .into_iter()
        .map(SharedProofWork::Single)
        .collect::<Vec<_>>();
    work.extend(
        request_groups
            .into_iter()
            .map(
                |((authority_set_id, queue_id), batch)| SharedProofWork::Batch {
                    authority_set_id,
                    queue_id,
                    latest: batch.latest,
                    batch_roots: batch.batch_roots,
                },
            ),
    );
    work
}

struct SharedBatchProofWork<T> {
    latest: T,
    batch_roots: Vec<(u32, H256)>,
}

impl<T: SharedProofRequestInfo> SharedBatchProofWork<T> {
    fn new(init: T) -> Self {
        Self {
            latest: init,
            batch_roots: Vec::new(),
        }
    }

    fn add_request(&mut self, request: T) {
        if request.block_number() > self.latest.block_number() {
            let previous_latest = std::mem::replace(&mut self.latest, request);
            self.batch_roots.push((
                previous_latest.block_number(),
                previous_latest.merkle_root(),
            ));
        } else {
            self.batch_roots
                .push((request.block_number(), request.merkle_root()));
        }
    }
}

#[derive(Clone)]
struct BatchProofRequest {
    block_number: u32,
    block_hash: H256,
    merkle_root: H256,
    inner_proof: ProofWithCircuitData,
    block_inclusion_proof: RawBlockInclusionProof,

    batch_roots: Vec<(u32, H256)>,
}

impl BatchProofRequest {
    fn new(init: Request) -> Self {
        Self {
            block_number: init.block_number,
            block_hash: init.block_hash,
            merkle_root: init.merkle_root,
            inner_proof: init.inner_proof,
            block_inclusion_proof: init.block_inclusion_proof,
            batch_roots: Vec::new(),
        }
    }

    /// Adds a request to the batch. If request is newer
    /// than the current request, it replaces the current one
    /// and adds the current merkle root to the batch roots.
    fn add_request(&mut self, request: Request) {
        if request.block_number > self.block_number {
            self.batch_roots.push((self.block_number, self.merkle_root));

            self.block_number = request.block_number;
            self.block_hash = request.block_hash;
            self.merkle_root = request.merkle_root;
            self.inner_proof = request.inner_proof;
            self.block_inclusion_proof = request.block_inclusion_proof;
        } else {
            self.batch_roots
                .push((request.block_number, request.merkle_root));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clear_current_shared_request, plan_shared_relayer_requests, record_current_shared_request,
        record_pending_request_counts, select_next_relayer_id, send_shared_response, Metrics,
        Response, SharedProofRequestInfo, SharedProofWork,
    };
    use crate::prover_interface::FinalProof;
    use primitive_types::H256;
    use prometheus::Registry;
    use tokio::sync::mpsc;
    use utils_prometheus::MeteredService;

    #[test]
    fn shared_scheduler_selects_highest_priority_relayer() {
        let selected =
            select_next_relayer_id([("testnet", 10, 0), ("mainnet", 100, 1), ("devnet", 50, 2)]);
        assert_eq!(selected.as_deref(), Some("mainnet"));
    }

    #[test]
    fn shared_scheduler_uses_fifo_for_equal_priority() {
        let selected =
            select_next_relayer_id([("second", 100, 2), ("first", 100, 1), ("third", 100, 3)]);
        assert_eq!(selected.as_deref(), Some("first"));
    }

    #[test]
    fn shared_metrics_expose_queue_and_current_work_by_relayer() {
        let metrics = Metrics::new();
        let registry = Registry::new();
        for source in metrics.get_sources() {
            registry.register(source).unwrap();
        }

        record_pending_request_counts(&metrics, [("mainnet", 2), ("testnet", 1)]);
        record_current_shared_request(&metrics, "mainnet", 100, 42, 1);

        let families = registry.gather();
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_pending_requests_by_relayer",
                "relayer",
                "mainnet",
            ),
            Some(2)
        );
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_pending_requests_by_relayer",
                "relayer",
                "testnet",
            ),
            Some(1)
        );
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_currently_processing_by_relayer",
                "relayer",
                "mainnet",
            ),
            Some(1)
        );
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_current_relayer_priority",
                "relayer",
                "mainnet",
            ),
            Some(100)
        );
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_current_root_block_by_relayer",
                "relayer",
                "mainnet",
            ),
            Some(42)
        );

        clear_current_shared_request(&metrics);
        let families = registry.gather();
        assert_eq!(
            gauge_value(
                &families,
                "merkle_root_relayer_currently_processing_by_relayer",
                "relayer",
                "mainnet",
            ),
            None
        );
    }

    #[test]
    fn shared_planner_processes_non_batched_requests_before_batches() {
        let work = plan_shared_relayer_requests(vec![
            test_request("batch-old", true, 20, 10, Some(1)),
            test_request("single-new", false, 30, 99, None),
            test_request("single-old", false, 10, 99, None),
            test_request("batch-new", true, 25, 10, Some(1)),
        ]);

        assert_eq!(work.len(), 3);
        assert_single(&work[0], "single-old", 10);
        assert_single(&work[1], "single-new", 30);
        assert_batch(&work[2], 1, 10, "batch-new", 25, &[(20, root(20))]);
    }

    #[test]
    fn shared_planner_batches_only_same_authority_set_and_queue() {
        let work = plan_shared_relayer_requests(vec![
            test_request("authority-one-old", true, 10, 7, Some(1)),
            test_request("authority-two", true, 40, 7, Some(2)),
            test_request("queue-eight", true, 30, 8, Some(1)),
            test_request("authority-one-new", true, 50, 7, Some(1)),
        ]);

        assert_eq!(work.len(), 3);
        assert_batch(&work[0], 1, 7, "authority-one-new", 50, &[(10, root(10))]);
        assert_batch(&work[1], 1, 8, "queue-eight", 30, &[]);
        assert_batch(&work[2], 2, 7, "authority-two", 40, &[]);
    }

    #[test]
    fn shared_batch_response_routes_only_to_originating_relayer() {
        let metrics = Metrics::new();
        let (origin_tx, mut origin_rx) = mpsc::unbounded_channel();
        let (_other_tx, mut other_rx) = mpsc::unbounded_channel::<Response>();

        send_shared_response(
            &origin_tx,
            Response::Batched {
                block_number: 42,
                merkle_root: root(42),
                proof: final_proof(42),
                batch_roots: vec![(41, root(41))],
            },
            &metrics,
            "mainnet",
        );

        match origin_rx
            .try_recv()
            .expect("originating relayer receives response")
        {
            Response::Batched {
                block_number,
                merkle_root,
                batch_roots,
                ..
            } => {
                assert_eq!(block_number, 42);
                assert_eq!(merkle_root, root(42));
                assert_eq!(batch_roots, vec![(41, root(41))]);
            }
            Response::Single { .. } => panic!("expected batched response"),
        }
        assert!(other_rx.try_recv().is_err());
        assert_eq!(metrics.dropped_responses.get(), 0);
    }

    #[derive(Debug)]
    struct TestRequest {
        name: &'static str,
        batch: bool,
        block_number: u32,
        merkle_root: H256,
        queue_id: u64,
        authority_set_id: Option<u64>,
    }

    impl SharedProofRequestInfo for TestRequest {
        fn is_batch(&self) -> bool {
            self.batch
        }

        fn block_number(&self) -> u32 {
            self.block_number
        }

        fn merkle_root(&self) -> H256 {
            self.merkle_root
        }

        fn queue_id(&self) -> u64 {
            self.queue_id
        }

        fn authority_set_id(&self) -> Option<u64> {
            self.authority_set_id
        }
    }

    fn test_request(
        name: &'static str,
        batch: bool,
        block_number: u32,
        queue_id: u64,
        authority_set_id: Option<u64>,
    ) -> TestRequest {
        TestRequest {
            name,
            batch,
            block_number,
            merkle_root: root(block_number),
            queue_id,
            authority_set_id,
        }
    }

    fn root(value: u32) -> H256 {
        H256::from_low_u64_be(value.into())
    }

    fn final_proof(block_number: u32) -> FinalProof {
        FinalProof {
            proof: vec![block_number as u8],
            block_number,
            merkle_root: [block_number as u8; 32],
        }
    }

    fn assert_single(work: &SharedProofWork<TestRequest>, name: &str, block_number: u32) {
        match work {
            SharedProofWork::Single(request) => {
                assert_eq!(request.name, name);
                assert_eq!(request.block_number, block_number);
            }
            SharedProofWork::Batch { .. } => panic!("expected single proof work"),
        }
    }

    fn assert_batch(
        work: &SharedProofWork<TestRequest>,
        authority_set_id: u64,
        queue_id: u64,
        latest_name: &str,
        latest_block: u32,
        batch_roots: &[(u32, H256)],
    ) {
        match work {
            SharedProofWork::Batch {
                authority_set_id: actual_authority_set_id,
                queue_id: actual_queue_id,
                latest,
                batch_roots: actual_batch_roots,
            } => {
                assert_eq!(*actual_authority_set_id, authority_set_id);
                assert_eq!(*actual_queue_id, queue_id);
                assert_eq!(latest.name, latest_name);
                assert_eq!(latest.block_number, latest_block);
                assert_eq!(actual_batch_roots, batch_roots);
            }
            SharedProofWork::Single(_) => panic!("expected batched proof work"),
        }
    }

    fn gauge_value(
        families: &[prometheus::proto::MetricFamily],
        name: &str,
        label_name: &str,
        label_value: &str,
    ) -> Option<i64> {
        families
            .iter()
            .find(|family| family.get_name() == name)?
            .get_metric()
            .iter()
            .find(|metric| {
                metric
                    .get_label()
                    .iter()
                    .any(|label| label.get_name() == label_name && label.get_value() == label_value)
            })
            .map(|metric| metric.get_gauge().get_value() as i64)
    }
}
