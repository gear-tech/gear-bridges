use crate::{
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
    prover_interface::{self, FinalProof},
};
use futures::executor::block_on;
use gear_rpc_client::{dto, GearApi};
use primitive_types::H256;
use prometheus::IntGauge;
use prover::proving::{GenesisConfig, ProofWithCircuitData};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

#[derive(Clone)]
pub struct Request {
    pub block_number: u32,
    pub block_hash: H256,
    pub merkle_root: H256,
    pub queue_id: u64,
    pub inner_proof: ProofWithCircuitData,
    pub batch: bool,
    pub block_finality: (H256, dto::BlockFinalityProof),
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

pub struct FinalityProverIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl FinalityProverIo {
    pub fn new(requests: UnboundedSender<Request>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
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
        block_finality: (H256, dto::BlockFinalityProof),
    ) -> bool {
        self.requests
            .send(Request {
                block_number,
                block_hash,
                merkle_root,
                inner_proof,
                queue_id,
                batch,
                block_finality,
            })
            .is_ok()
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
    api_provider: ApiProviderConnection,
    genesis_config: GenesisConfig,

    count_thread: Option<usize>,

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
    ) -> Self {
        Self {
            api_provider,

            genesis_config,

            count_thread,

            metrics: Metrics::new(),
        }
    }

    pub fn run(mut self) -> FinalityProverIo {
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (res_tx, res_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = FinalityProverIo::new(req_tx, res_rx);

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
        let gear_api = self.api_provider.client();

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
                                &gear_api,
                                request.block_number,
                                request.block_hash,
                                request.merkle_root,
                                request.inner_proof,
                                request.block_finality,
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

                let authority_set_id = gear_api.authority_set_id(request.block_hash).await?;
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
                        &gear_api,
                        request.block_number,
                        request.block_hash,
                        request.merkle_root,
                        request.inner_proof,
                        request.block_finality,
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
                    block_finality,
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
                        &gear_api,
                        block_number,
                        block_hash,
                        merkle_root,
                        inner_proof,
                        block_finality,
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
        gear_api: &GearApi,
        block_number: u32,
        block_hash: H256,
        merkle_root: H256,
        inner_proof: ProofWithCircuitData,
        block_finality: (H256, dto::BlockFinalityProof),
    ) -> anyhow::Result<FinalProof> {
        log::info!("Generating merkle root proof for block #{block_number}");

        log::info!("Proving merkle root({merkle_root}) presence in block #{block_number}");

        let start = Instant::now();
        let proof = prover_interface::prove_final(
            gear_api,
            inner_proof,
            self.genesis_config,
            block_hash,
            self.count_thread,
            Some(block_finality),
        )
        .await?;
        let elapsed = start.elapsed().as_secs_f64();
        log::info!(
            "Proof for {merkle_root} generated (block #{block_number}) in {elapsed:.3} seconds",
        );

        self.metrics.last_proof_time.set(elapsed.ceil() as i64);
        if self.metrics.min_proof_time.get() == 0
            || elapsed < self.metrics.min_proof_time.get() as f64
        {
            self.metrics.min_proof_time.set(elapsed.ceil() as i64);
        }
        if elapsed > self.metrics.max_proof_time.get() as f64 {
            self.metrics.max_proof_time.set(elapsed.ceil() as i64);
        }

        Ok(proof)
    }
}

#[derive(Clone)]
struct BatchProofRequest {
    block_number: u32,
    block_hash: H256,
    merkle_root: H256,
    inner_proof: ProofWithCircuitData,
    block_finality: (H256, dto::BlockFinalityProof),

    batch_roots: Vec<(u32, H256)>,
}

impl BatchProofRequest {
    fn new(init: Request) -> Self {
        Self {
            block_number: init.block_number,
            block_hash: init.block_hash,
            merkle_root: init.merkle_root,
            inner_proof: init.inner_proof,
            block_finality: init.block_finality,
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
            self.block_finality = request.block_finality;
        } else {
            self.batch_roots
                .push((request.block_number, request.merkle_root));
        }
    }
}
