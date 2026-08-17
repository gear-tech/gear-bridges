use crate::{
    cli::GearEthCoreArgs,
    merkle_roots::{authority_set_sync::AuthoritySetSyncIo, prover::FinalityProverIo},
    message_relayer::common::{
        gear::block_listener::BlockListener,
        web_request::{MerkleRootsRequest, MerkleRootsResponse},
        GearBlock,
    },
    proof_storage::ProofStorageError,
    prover_interface::FinalProof,
    rpc,
};
use ::prover::proving::{GenesisConfig, ProofWithCircuitData};
use anyhow::Context;
use ethereum_client::EthApi;
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::dto::RawBlockInclusionProof;
use primitive_types::{H256, U256};
use prometheus::IntGauge;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use storage::MerkleRootStorage;
use submitter::SubmitterIo;
use tokio::{
    sync::{
        broadcast::{error::RecvError, Receiver},
        mpsc::UnboundedReceiver,
    },
    time::{Interval, MissedTickBehavior},
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub mod authority_set_sync;
pub mod prover;
pub mod storage;
pub mod submitter;

const MERKLE_ROOT_SUPERVISOR_INTERVAL: Duration = Duration::from_secs(15 * 60);
const MERKLE_ROOT_SUPERVISOR_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PENDING_HTTP_REQUESTS_PER_ROOT: usize = 128;
type PendingContinuation = ((u32, H256), u32);

enum EthereumRootRead {
    Fetched(Option<H256>),
    RetryLater,
}

pub struct Relayer {
    merkle_roots: MerkleRootRelayer,
    authority_set_sync: authority_set_sync::AuthoritySetSync,
    prover: ProverSource,
    submitter: submitter::MerkleRootSubmitter,
    block_listener: BlockListener,

    eth_api: EthApi,
    http: UnboundedReceiver<MerkleRootsRequest>,
}

enum ProverSource {
    Owned(prover::FinalityProver),
    External(FinalityProverIo),
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        let mut sources: Vec<Box<dyn prometheus::core::Collector>> = Vec::new();
        sources.extend(self.authority_set_sync.get_sources());
        sources.extend(self.block_listener.get_sources());
        sources.extend(self.merkle_roots.get_sources());
        sources.extend(self.submitter.get_sources());
        if let ProverSource::Owned(prover) = &self.prover {
            sources.extend(prover.get_sources());
        }
        sources
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        http: UnboundedReceiver<MerkleRootsRequest>,
        storage: Arc<MerkleRootStorage>,
        options: MerkleRootRelayerOptions,
    ) -> Self {
        let block_listener = BlockListener::new_for_relayer(
            api_provider.clone(),
            storage.clone(),
            options.relayer_id.clone(),
        );

        let authority_set_sync = authority_set_sync::AuthoritySetSync::new(
            api_provider.clone(),
            storage.proofs.clone(),
            options.genesis_config,
            options.count_thread,
            options.relayer_id.clone(),
            options.priority,
            options.shared_authority_set_sync.clone(),
        )
        .await;

        let prover = ProverSource::Owned(prover::FinalityProver::new(
            api_provider.clone(),
            options.genesis_config,
            options.count_thread,
            options.gnark_data_path.clone(),
        ));

        let submitter = submitter::MerkleRootSubmitter::new(
            eth_api.clone(),
            storage.clone(),
            options.confirmations,
            options.relayer_id.clone(),
        );
        let merkle_roots = MerkleRootRelayer::new(api_provider, storage, options).await;

        Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,

            eth_api,
            http,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_with_prover_io(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        http: UnboundedReceiver<MerkleRootsRequest>,
        storage: Arc<MerkleRootStorage>,
        options: MerkleRootRelayerOptions,
        prover: FinalityProverIo,
    ) -> Self {
        let block_listener = BlockListener::new_for_relayer(
            api_provider.clone(),
            storage.clone(),
            options.relayer_id.clone(),
        );

        let authority_set_sync = authority_set_sync::AuthoritySetSync::new(
            api_provider.clone(),
            storage.proofs.clone(),
            options.genesis_config,
            options.count_thread,
            options.relayer_id.clone(),
            options.priority,
            options.shared_authority_set_sync.clone(),
        )
        .await;

        let submitter = submitter::MerkleRootSubmitter::new(
            eth_api.clone(),
            storage.clone(),
            options.confirmations,
            options.relayer_id.clone(),
        );
        let merkle_roots = MerkleRootRelayer::new(api_provider, storage, options).await;

        Self {
            merkle_roots,
            authority_set_sync,
            prover: ProverSource::External(prover),
            submitter,
            block_listener,

            eth_api,
            http,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,

            eth_api,
            http,
        } = self;

        let [blocks0, blocks1] = block_listener.run().await;

        let authority_set_sync = authority_set_sync.run(blocks1);
        let prover = match prover {
            ProverSource::Owned(prover) => prover.run(),
            ProverSource::External(prover) => prover,
        };
        let submitter = submitter.run();

        merkle_roots
            .run(
                blocks0,
                submitter,
                prover,
                authority_set_sync,
                http,
                eth_api,
            )
            .await
    }
}

impl_metered_service!(
    struct Metrics {
        last_confirmed_block: IntGauge = IntGauge::new(
            "merkle_root_relayer_last_submitted_block",
            "Block number of the last confirmed merkle root"
        ),
        first_pending_timestamp: IntGauge = IntGauge::new(
            "merkle_root_relayer_first_pending_timestamp",
            "Timestamp of the first pending merkle root"
        ),
        batch_delay: IntGauge = IntGauge::new(
            "merkle_root_relayer_batch_delay",
            "Delay until the current batch is processed (in seconds)"
        ),
        batch_size: IntGauge = IntGauge::new(
            "merkle_root_relayer_batch_size",
            "Current size of the merkle root batch"
        ),
        total_merkle_roots: IntGauge = IntGauge::new(
            "merkle_root_relayer_total_roots",
            "Total number of merkle roots in storage"
        ),
        total_waiting_for_authority_set_sync: IntGauge = IntGauge::new(
            "merkle_root_relayer_waiting_for_authority_set_sync",
            "Number of blocks waiting for authority set sync"
        ),
    }
);

struct DeferredStartupProof {
    block_number: u32,
    block_hash: H256,
    merkle_root: H256,
    authority_set_id: u64,
    queue_id: u64,
    batch: bool,
    block_inclusion_proof: RawBlockInclusionProof,
}

fn defer_startup_proof(
    roots: &mut HashMap<(u32, H256), MerkleRoot>,
    deferred: &mut Vec<DeferredStartupProof>,
    key: (u32, H256),
    authority_set_id: u64,
) {
    let root = roots.get_mut(&key).expect("persisted root was reinstated");
    root.single_proof_in_flight = false;
    deferred.push(DeferredStartupProof {
        block_number: root.block_number,
        block_hash: root.block_hash,
        merkle_root: key.1,
        authority_set_id,
        queue_id: root.queue_id,
        batch: root.batch,
        block_inclusion_proof: root.block_inclusion_proof.clone(),
    });
}

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,

    storage: Arc<MerkleRootStorage>,

    roots: HashMap<(u32, H256), MerkleRoot>,

    /// Set of blocks that are waiting for authority set sync.
    waiting_for_authority_set_sync: BTreeMap<u64, Vec<GearBlock>>,

    last_confirmed_block: Option<u32>,
    last_confirmed_timestamp_ms: Option<u64>,
    max_block_distance: Option<u32>,
    first_pending_timestamp: Option<Instant>,
    queued_root_timestamps: VecDeque<Instant>,
    merkle_root_batch: Vec<PendingMerkleRoot>,
    pending_continuations: VecDeque<PendingContinuation>,
    pending_submissions: VecDeque<(u32, H256)>,
    pending_submission_keys: HashSet<(u32, H256)>,

    options: MerkleRootRelayerOptions,

    save_interval: Interval,
    main_interval: Interval,
    supervisor_interval: Interval,

    metrics: Metrics,
}

impl MeteredService for MerkleRootRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MerkleRootRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        storage: Arc<MerkleRootStorage>,
        options: MerkleRootRelayerOptions,
    ) -> MerkleRootRelayer {
        let mut save_interval = tokio::time::interval(options.save_interval);
        save_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut main_interval = tokio::time::interval(options.check_interval);
        main_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut supervisor_interval = tokio::time::interval(MERKLE_ROOT_SUPERVISOR_INTERVAL);
        supervisor_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        MerkleRootRelayer {
            api_provider,

            roots: HashMap::new(),
            storage,

            waiting_for_authority_set_sync: BTreeMap::new(),

            last_confirmed_block: None,
            last_confirmed_timestamp_ms: None,
            max_block_distance: None,
            first_pending_timestamp: None,
            queued_root_timestamps: VecDeque::with_capacity(8),
            merkle_root_batch: Vec::with_capacity(8),
            pending_continuations: VecDeque::new(),
            pending_submissions: VecDeque::new(),
            pending_submission_keys: HashSet::new(),

            options,
            save_interval,
            main_interval,
            supervisor_interval,

            metrics: Metrics::new(),
        }
    }

    fn prune_old_timestamps(&mut self) {
        let cutoff_time = Instant::now() - self.options.spike_config.window;

        while let Some(&timestamp) = self.queued_root_timestamps.front() {
            if timestamp < cutoff_time {
                self.queued_root_timestamps.pop_front();
            } else {
                break;
            }
        }
    }

    fn queue_submission(&mut self, key: (u32, H256)) {
        if self.pending_submission_keys.insert(key) {
            self.pending_submissions.push_back(key);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        mut self,
        mut blocks_rx: Receiver<GearBlock>,
        mut submitter: SubmitterIo,
        mut prover: FinalityProverIo,
        mut authority_set_sync: AuthoritySetSyncIo,
        mut http: UnboundedReceiver<MerkleRootsRequest>,

        mut eth_api: EthApi,
    ) -> anyhow::Result<()> {
        let relayer_id = self.options.relayer_id.clone();
        log::info!("Starting merkle root relayer {relayer_id}");
        let mut roots = match self.storage.load().await {
            Ok(roots) => roots,
            Err(err)
                if err
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
            {
                log::info!(
                    "Merkle root relayer {relayer_id}: no persisted merkle-root state found"
                );
                Default::default()
            }
            Err(err) => {
                return Err(err).context(format!(
                    "Merkle root relayer {relayer_id}: failed to load merkle roots from storage"
                ));
            }
        };
        let gear_api = self.api_provider.client();
        let mut last_sealed = match self.options.last_sealed {
            Some(era) => era,
            None => {
                let block = gear_api
                    .latest_finalized_block()
                    .await
                    .context("Failed to get latest finalized block during startup")?;
                gear_api
                    .authority_set_id(block)
                    .await
                    .context("Failed to get authority set id for latest finalized block")?
            }
        };

        log::info!(
            "Merkle root relayer {relayer_id}: ensuring authority sets are synced on startup"
        );
        if !authority_set_sync.initialize() {
            return Err(anyhow::anyhow!(
                "Merkle root relayer {relayer_id}: failed to enqueue authority set sync startup job"
            ));
        }

        let gear_api = self.api_provider.client();
        let mut deferred_startup_proofs = Vec::new();

        for ((block_number, hash), merkle_root) in roots.drain() {
            let block_hash = merkle_root.block_hash;

            let mut reinstate = |status: MerkleRootStatus| {
                self.roots.insert(
                    (block_number, hash),
                    MerkleRoot {
                        queue_id: merkle_root.queue_id,
                        block_number,
                        block_hash,
                        status: status.clone(),
                        message_nonces: Vec::new(),
                        proof: merkle_root.proof.clone(),
                        covered_roots: merkle_root.covered_roots.clone(),
                        covered_source_blocks: merkle_root.covered_source_blocks.clone(),
                        continuation_source_blocks: merkle_root.continuation_source_blocks.clone(),
                        batch: merkle_root.batch,
                        single_proof_in_flight: matches!(status, MerkleRootStatus::GenerateProof)
                            && !merkle_root.batch,
                        http_requests: Vec::new(),
                        block_inclusion_proof: merkle_root.block_inclusion_proof.clone(),
                    },
                );
            };

            match &merkle_root.status {
                // most likely will need to wait for era sealing rather than authority set sync
                MerkleRootStatus::WaitForAuthoritySetSync(id, _) => {
                    log::info!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} is waiting for authority set sync with id {id}"
                    );

                    let proof_target_hash = merkle_root.block_inclusion_proof.block_hash;
                    let block = gear_api.get_block_at(proof_target_hash).await?;
                    let block = GearBlock::from_subxt_block(&gear_api, block).await?;

                    match self
                        .storage
                        .proofs
                        .get_proof_for_authority_set_id(*id)
                        .await
                    {
                        Ok(_) => {
                            reinstate(MerkleRootStatus::GenerateProof);
                            defer_startup_proof(
                                &mut self.roots,
                                &mut deferred_startup_proofs,
                                (block_number, hash),
                                *id,
                            );
                        }
                        Err(_) => {
                            log::warn!("Merkle root relayer {relayer_id}: authority set proof for #{id} not found, waiting for authority set sync");

                            reinstate(MerkleRootStatus::WaitForAuthoritySetSync(
                                *id,
                                block.number(),
                            ));

                            // if authority set is older than last sealed era we need to seal this
                            // authority set first.
                            if *id <= last_sealed {
                                last_sealed = id.saturating_sub(1);
                            }

                            let force_sync =
                                match self.storage.proofs.get_latest_authority_set_id().await {
                                    Some(latest) => *id > latest,
                                    None => true,
                                } && *id > last_sealed;

                            let waiting =
                                self.waiting_for_authority_set_sync.entry(*id).or_default();
                            if waiting.is_empty()
                                && force_sync
                                && !authority_set_sync.send(block.clone())
                            {
                                return Err(anyhow::anyhow!(
                                    "Merkle root relayer {relayer_id}: authority set sync connection closed during startup recovery"
                                ));
                            }
                            waiting.push(block);
                        }
                    }
                }

                MerkleRootStatus::GenerateProof => {
                    reinstate(MerkleRootStatus::GenerateProof);
                    defer_startup_proof(
                        &mut self.roots,
                        &mut deferred_startup_proofs,
                        (block_number, hash),
                        merkle_root.block_inclusion_proof.required_authority_set_id,
                    );

                    log::info!(
                        "Merkle root relayer {relayer_id}: deferring persisted proof for merkle root {hash} at block #{block_number} until after startup supervision"
                    );
                }

                MerkleRootStatus::SubmitProof if merkle_root.proof.is_some() => {
                    log::info!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} is waiting for proof submission"
                    );
                    reinstate(MerkleRootStatus::SubmitProof);
                    self.queue_submission((block_number, hash));
                }

                MerkleRootStatus::SubmitProof => {
                    return Err(anyhow::anyhow!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} has no proof in SubmitProof state"
                    ));
                }

                MerkleRootStatus::Failed(err) => {
                    reinstate(MerkleRootStatus::Failed(err.clone()));

                    log::error!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} failed: {err}"
                    );
                }

                MerkleRootStatus::Finalized => {
                    reinstate(MerkleRootStatus::Finalized);

                    log::info!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} is finalized"
                    );
                }
            }
        }
        self.pending_continuations
            .extend(recover_finalized_continuations(&self.roots));

        self.initialize_contract_cursor(&mut eth_api).await?;
        // Consume the interval's immediate first tick before supervision so a failed
        // startup probe can schedule an actual short retry instead of losing it here.
        self.supervisor_interval.tick().await;
        self.supervise_contract_state(&mut prover, &mut authority_set_sync, &mut eth_api)
            .await?;

        for deferred in deferred_startup_proofs {
            let key = (deferred.block_number, deferred.merkle_root);
            if self
                .roots
                .get(&key)
                .is_some_and(|root| root.single_proof_in_flight)
            {
                continue;
            }
            let inner_proof = self
                .storage
                .proofs
                .get_proof_for_authority_set_id(deferred.authority_set_id)
                .await
                .with_context(|| format!("Proof for authority set #{} not found, please clean-up your storage and restart relayer", deferred.authority_set_id))?;
            if !deferred.batch {
                self.roots
                    .get_mut(&key)
                    .expect("persisted root was reinstated")
                    .single_proof_in_flight = true;
            }
            if !prover.prove(
                deferred.block_number,
                deferred.block_hash,
                deferred.merkle_root,
                inner_proof,
                deferred.queue_id,
                deferred.batch,
                deferred.block_inclusion_proof,
            ) {
                return Err(anyhow::anyhow!(
                    "Merkle root relayer {relayer_id}: prover connection closed while resuming persisted proof"
                ));
            }
        }

        if let Err(err) = self
            .run_inner(
                &mut submitter,
                &mut prover,
                &mut blocks_rx,
                &mut authority_set_sync,
                &mut http,
                &mut eth_api,
            )
            .await
        {
            log::error!("Merkle root relayer {relayer_id} encountered an error: {err}");
            Err(err)
        } else {
            log::warn!(
                "Merkle root relayer {relayer_id}: Gear block listener connection closed, exiting"
            );
            Ok(())
        }
    }

    async fn initialize_contract_cursor(&mut self, eth_api: &mut EthApi) -> anyhow::Result<()> {
        let relayer_id = &self.options.relayer_id;
        let max_block_number = rpc::retry_eth(
            eth_api,
            "read finalized MessageQueue max block number",
            |api| async move { api.finalized_max_block_number().await },
        )
        .await?;
        let max_block_distance = rpc::retry_eth(
            eth_api,
            "read MessageQueue max block distance",
            |api| async move { api.max_block_distance().await },
        )
        .await?;
        let client = self.api_provider.client();
        let last_block_hash = client.latest_finalized_block().await?;
        let last_block = client.block_hash_to_number(last_block_hash).await?;
        let max_block_number_in_storage = self
            .roots
            .values()
            .filter(|root| {
                root.proof.is_some() && matches!(root.status, MerkleRootStatus::Finalized)
            })
            .map(|root| root.block_number)
            .max();
        let confirmed_block = max_block_number;
        let confirmed_block_hash = client.block_number_to_hash(confirmed_block).await?;
        let confirmed_timestamp_ms = client.fetch_timestamp(confirmed_block_hash).await?;

        log::info!("Merkle root relayer {relayer_id}: latest finalized block is #{last_block}, max block number in Ethereum MessageQueue contract is #{max_block_number} (MAX_BLOCK_DISTANCE={max_block_distance})");
        if let Some(max_stored) = max_block_number_in_storage {
            if max_stored > confirmed_block {
                log::warn!(
                    "Merkle root relayer {relayer_id}: ignoring local finalized root at block #{max_stored} because Ethereum MessageQueue is only at block #{confirmed_block}"
                );
            } else {
                log::info!(
                    "Merkle root relayer {relayer_id}: max finalized merkle root in storage is at block #{max_stored}"
                );
            }
        } else {
            log::info!("Merkle root relayer {relayer_id}: no finalized merkle roots in storage");
        }

        self.last_confirmed_block = Some(confirmed_block);
        self.last_confirmed_timestamp_ms = Some(confirmed_timestamp_ms);
        self.max_block_distance = Some(max_block_distance);
        Ok(())
    }

    async fn supervise_contract_state(
        &mut self,
        prover: &mut FinalityProverIo,
        authority_set_sync: &mut AuthoritySetSyncIo,
        eth_api: &mut EthApi,
    ) -> anyhow::Result<()> {
        let relayer_id = self.options.relayer_id.clone();
        let client = self.api_provider.client();
        let latest_block_hash = client.latest_finalized_block().await?;
        let latest_block = client.block_hash_to_number(latest_block_hash).await?;
        let latest_timestamp_ms = client.fetch_timestamp(latest_block_hash).await?;
        let latest_queue_state = client
            .fetch_authenticated_queue_merkle_root(latest_block_hash)
            .await?;
        let confirmed_block = self
            .last_confirmed_block
            .context("Ethereum MessageQueue max block number is not initialized")?;
        let confirmed_block_hash = client.block_number_to_hash(confirmed_block).await?;
        let confirmed_queue_state = client
            .fetch_authenticated_queue_merkle_root(confirmed_block_hash)
            .await?;

        if self.last_confirmed_timestamp_ms.is_none() {
            self.last_confirmed_timestamp_ms =
                Some(client.fetch_timestamp(confirmed_block_hash).await?);
        }
        let threshold = critical_timeout_reached(
            self.options.critical_threshold,
            self.last_confirmed_block,
            self.last_confirmed_timestamp_ms,
            latest_timestamp_ms,
        );
        let Some((last_confirmed_block, threshold)) = threshold else {
            log::debug!(
                "Merkle root relayer {relayer_id} supervisor: critical threshold is not reached at latest finalized Vara block #{latest_block}, skipping forced proof generation"
            );
            return Ok(());
        };

        if !queue_state_changed(confirmed_queue_state, latest_queue_state) {
            log::trace!(
                "Merkle root relayer {relayer_id} supervisor: Vara queue state has not changed since Ethereum MessageQueue block #{confirmed_block}, skipping proof generation"
            );
            return Ok(());
        }

        let Some(target_limit) = recovery_target_limit(
            self.last_confirmed_block,
            self.max_block_distance,
            latest_block,
        ) else {
            log::debug!(
                "Merkle root relayer {relayer_id} supervisor: no finalized Vara block advances MessageQueue block #{confirmed_block}"
            );
            return Ok(());
        };

        let block = self.signed_block_at_or_before(target_limit).await?;
        let block_number = block.number();
        if block_number <= confirmed_block {
            log::warn!(
                "Merkle root relayer {relayer_id} supervisor: no signed Vara block advances MessageQueue block #{confirmed_block} inside the bound ending at #{target_limit}"
            );
            return Ok(());
        }
        let block_hash = block.hash();
        let block_timestamp_ms = client.fetch_timestamp(block_hash).await?;
        let (_, merkle_root) = client
            .fetch_authenticated_queue_merkle_root(block_hash)
            .await?;

        let eth_root = match self
            .read_finalized_merkle_root(eth_api, block_number)
            .await?
        {
            EthereumRootRead::Fetched(root) => root,
            EthereumRootRead::RetryLater => {
                self.supervisor_interval
                    .reset_after(MERKLE_ROOT_SUPERVISOR_RETRY_INTERVAL);
                return Ok(());
            }
        };
        if eth_root == Some(merkle_root) {
            log::info!(
                "Merkle root relayer {relayer_id} supervisor: finalized Ethereum state already has merkle root {merkle_root} for bounded Vara block #{block_number}"
            );
            self.last_confirmed_block = Some(block_number);
            self.last_confirmed_timestamp_ms = Some(block_timestamp_ms);
            return Ok(());
        }

        log::warn!(
            "Merkle root relayer {relayer_id} supervisor: MessageQueue block #{last_confirmed_block} is stale by at least {threshold:?}; scheduling recovery at signed Vara block #{block_number} within upper bound #{target_limit} (latest finalized #{latest_block})"
        );
        if let Some(eth_root) = eth_root {
            log::warn!(
                "Merkle root relayer {relayer_id} supervisor: Ethereum has merkle root {eth_root} for bounded Vara block #{block_number}, but Vara queue root is {merkle_root}"
            );
        }

        if let Some((selected_queue_id, selected_merkle_root)) = self
            .try_proof_merkle_root(
                prover,
                authority_set_sync,
                block,
                ProofTarget::Recovery,
                Batch::No,
                Priority::No,
                ForceGeneration::Yes,
            )
            .await?
        {
            log::info!(
                "Merkle root relayer {relayer_id} supervisor: bounded proof request selected queue #{selected_queue_id}, merkle root {selected_merkle_root} at Vara block #{block_number}"
            );
        }
        Ok(())
    }

    async fn read_finalized_merkle_root(
        &self,
        eth_api: &mut EthApi,
        block_number: u32,
    ) -> anyhow::Result<EthereumRootRead> {
        let relayer_id = &self.options.relayer_id;
        match eth_api.read_finalized_merkle_root(block_number).await {
            Ok(root) => return Ok(EthereumRootRead::Fetched(root.map(H256::from))),
            Err(err) if rpc::classify_ethereum_error(&err) == rpc::RetryDecision::Retry => {
                log::warn!(
                    "Merkle root relayer {relayer_id} supervisor: recoverable Ethereum RPC error while reading merkle root for block #{block_number}: {err}. Reconnecting"
                );
            }
            Err(err) => return Err(err.into()),
        }

        let reconnected = match eth_api.reconnect().await {
            Ok(reconnected) => reconnected,
            Err(err) if rpc::classify_ethereum_error(&err) == rpc::RetryDecision::Retry => {
                log::warn!(
                    "Merkle root relayer {relayer_id} supervisor: Ethereum reconnect failed: {err}. Retrying supervisor in {MERKLE_ROOT_SUPERVISOR_RETRY_INTERVAL:?}"
                );
                return Ok(EthereumRootRead::RetryLater);
            }
            Err(err) => return Err(err.into()),
        };
        *eth_api = reconnected;

        match eth_api.read_finalized_merkle_root(block_number).await {
            Ok(root) => Ok(EthereumRootRead::Fetched(root.map(H256::from))),
            Err(err) if rpc::classify_ethereum_error(&err) == rpc::RetryDecision::Retry => {
                log::warn!(
                    "Merkle root relayer {relayer_id} supervisor: Ethereum merkle-root read still unavailable after reconnect: {err}. Retrying supervisor in {MERKLE_ROOT_SUPERVISOR_RETRY_INTERVAL:?}"
                );
                Ok(EthereumRootRead::RetryLater)
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn signed_block_after(&self, block_number: u32) -> anyhow::Result<GearBlock> {
        let api = self.api_provider.client();
        let (justification, _) = api
            .grandpa_prove_finality(block_number)
            .await
            .with_context(|| {
                format!("Failed to fetch finality proof after block #{block_number}")
            })?;
        let signed_block_number = justification.commit.target_number;
        let signed_block_hash = H256::from(justification.commit.target_hash.0);
        log::info!(
            "Merkle root relayer {} supervisor: requested block #{block_number}, signed block id = ({signed_block_number}, {signed_block_hash:?})",
            self.options.relayer_id
        );
        GearBlock::from_justification(&api, justification).await
    }

    async fn signed_block_at_or_before(&self, max_block_number: u32) -> anyhow::Result<GearBlock> {
        let mut requested = max_block_number;
        for _ in 0..8 {
            let block = self.signed_block_after(requested).await?;
            if block.number() <= max_block_number {
                return Ok(block);
            }

            let overshoot = block.number() - max_block_number;
            let next = requested.saturating_sub(overshoot.max(1));
            if next == requested {
                break;
            }
            requested = next;
        }

        Err(anyhow::anyhow!(
            "Unable to find a GRANDPA-signed block at or before contract limit #{max_block_number}"
        ))
    }

    async fn run_inner(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,

        http: &mut UnboundedReceiver<MerkleRootsRequest>,
        eth_api: &mut EthApi,
    ) -> anyhow::Result<()> {
        loop {
            let result = self
                .process(
                    submitter,
                    prover,
                    blocks_rx,
                    authority_set_sync,
                    http,
                    eth_api,
                )
                .await;

            match result {
                Ok(true) => continue,
                Ok(false) => {
                    return Err(anyhow::anyhow!(
                        "merkle root relayer {} component channel closed",
                        self.options.relayer_id
                    ));
                }
                Err(err) => {
                    log::error!(
                        "Merkle root relayer {}: error processing blocks: {err}",
                        self.options.relayer_id
                    );
                    return Err(err);
                }
            }
        }
    }

    async fn process(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,

        http: &mut UnboundedReceiver<MerkleRootsRequest>,
        eth_api: &mut EthApi,
    ) -> anyhow::Result<bool> {
        let client = self.api_provider.client();
        let pending_submission = self
            .pending_submissions
            .front()
            .map(|&(block_number, merkle_root)| {
                let proof = self
                    .roots
                    .get(&(block_number, merkle_root))
                    .and_then(|root| root.proof.clone())
                    .with_context(|| {
                        format!(
                            "Queued merkle root {merkle_root} for block #{block_number} has no proof"
                        )
                    })?;
                Ok::<_, anyhow::Error>(submitter::Request {
                    era: None,
                    merkle_root_block: block_number,
                    merkle_root,
                    proof,
                })
            })
            .transpose()?;
        let has_pending_submission = pending_submission.is_some();
        let submission_tx = submitter.request_sender();
        tokio::select! {
            submission = async {
                submission_tx
                    .send(pending_submission.expect("submission branch requires queued work"))
                    .await
            }, if has_pending_submission => {
                if submission.is_err() {
                    log::warn!(
                        "Merkle root relayer {}: proof submitter connection closed, exiting",
                        self.options.relayer_id
                    );
                    return Ok(false);
                }
                let key = self
                    .pending_submissions
                    .pop_front()
                    .expect("submitted work was read from the pending queue");
                self.pending_submission_keys.remove(&key);
            }
            _ = self.save_interval.tick() => {
                log::trace!("60 seconds passed, saving current state");
                if let Err(err) = self.storage.save(&self.roots).await {
                    log::error!(
                        "Merkle root relayer {}: failed to save block state: {err:?}",
                        self.options.relayer_id
                    );
                }
            }

            _ = self.main_interval.tick() => {
                // prune old timestamps to not trigger spike when not necessary
                self.prune_old_timestamps();

                // update metrics
                self.metrics.total_merkle_roots.set(self.roots.len() as i64);
                self.metrics.total_waiting_for_authority_set_sync.set(self.waiting_for_authority_set_sync.values().map(|v| v.len()).sum::<usize>() as i64);
                self.metrics.last_confirmed_block.set(self.last_confirmed_block.unwrap_or(0) as i64);
                self.metrics.first_pending_timestamp.set(self.first_pending_timestamp.map(|t| t.elapsed().as_secs() as i64).unwrap_or(0));
                self.metrics.batch_size.set(self.merkle_root_batch.len() as i64);
                if let Some(first) = self.first_pending_timestamp {
                    self.metrics.batch_delay.set((self.options.spike_config.timeout.as_secs() as i64) - (first.elapsed().as_secs() as i64));
                } else {
                    self.metrics.batch_delay.set(0);
                }


                let has_priority = self.merkle_root_batch.iter().any(|root| root.priority);
                let timeout = if has_priority {
                    self.options.spike_config.priority_timeout
                } else {
                    self.options.spike_config.timeout
                };
                let is_spike = self.merkle_root_batch.iter().map(|root| root.nonces_count).sum::<usize>() >= self.options.spike_config.threshold;
                let is_timeout = self.first_pending_timestamp
                    .is_some_and(|t| t.elapsed() >= timeout);

                if is_spike || is_timeout {
                    // consume the timestamp to not trigger timeout again immediately.
                    self.first_pending_timestamp.take();
                    let batch_size = self.merkle_root_batch.len();
                    if batch_size == 0 {
                        return Ok(true);
                    }
                    log::info!("Merkle root relayer {}: triggering proof generation. Batch size: {batch_size}, Reason: Spike={is_spike}, Timeout={is_timeout}", self.options.relayer_id);
                    // do not group blocks by authority set id, prover will do this for us.
                    for pending in self.merkle_root_batch.drain(..) {
                        let merkle_root = &self.roots[&(pending.block_number, pending.merkle_root)];
                        if !prover.prove(
                            pending.block_number,
                            pending.block_hash,
                            pending.merkle_root,
                            pending.inner_proof,
                            pending.queue_id,
                            /* request is part of the batch: */
                            true,
                            merkle_root.block_inclusion_proof.clone(),
                        ) {
                            log::warn!(
                                "Merkle root relayer {}: prover connection closed, exiting",
                                self.options.relayer_id
                            );
                            return Ok(false);
                        }
                    }
                }
            }

            _ = self.supervisor_interval.tick() => {
                self.supervise_contract_state(prover, authority_set_sync, eth_api).await?;
            }

            req = http.recv() => {
                match req {
                    Some(req) => {
                        match req {
                            MerkleRootsRequest::GetMerkleRootProof {
                                block_number,
                                response
                            } => {
                                // Exact requests may only use a proof whose target block and root
                                // match the stored root metadata. Batched proofs target a later
                                // anchor and must fall through to exact proof generation.
                                if let Some((&(_, merkle_root), root)) = self.roots.iter().find(
                                    |((root_block_number, root_merkle_root), root)| {
                                        *root_block_number == block_number
                                            && root.proof.as_ref().is_some_and(|proof| {
                                                proof.block_number == *root_block_number
                                                    && H256::from(proof.merkle_root)
                                                        == *root_merkle_root
                                            })
                                    },
                                ) {
                                    if let MerkleRootStatus::Finalized = root.status {
                                        let proof = root.proof.as_ref().expect("proof availability is checked above");
                                        if response.send(MerkleRootsResponse::MerkleRootProof {
                                            proof: proof.proof.clone(),
                                            proof_block_number: proof.block_number,
                                            block_number: root.block_number,
                                            block_hash: root.block_hash,
                                            merkle_root,
                                        }).is_err() {
                                            log::debug!("Merkle root relayer {}: HTTP client disconnected before response", self.options.relayer_id);
                                        }
                                        return Ok(true);
                                    }
                                }

                                let api = self.api_provider.client();
                                let block_hash = api.block_number_to_hash(block_number).await?;
                                let block = api.get_block_at(block_hash).await?;
                                let block = GearBlock::from_subxt_block(&client, block).await?;

                                match self.try_proof_merkle_root(prover, authority_set_sync, block, ProofTarget::Exact, Batch::No, Priority::Yes, ForceGeneration::Yes).await {
                                    Ok(Some((_, merkle_root))) => {
                                        if let Some(root) =
                                            self.roots.get_mut(&(block_number, merkle_root))
                                        {
                                            root.http_requests
                                                .retain(|request| !request.is_closed());
                                            if root.http_requests.len()
                                                >= MAX_PENDING_HTTP_REQUESTS_PER_ROOT
                                            {
                                                response
                                                    .send(MerkleRootsResponse::Failed {
                                                        message: format!(
                                                            "Too many pending proof requests for block #{block_number}"
                                                        ),
                                                    })
                                                    .ok();
                                            } else {
                                                root.http_requests.push(response);
                                            }
                                        } else {
                                            response
                                                .send(
                                                    MerkleRootsResponse::NoMerkleRootOnBlock {
                                                        block_number,
                                                    },
                                                )
                                                .ok();
                                        }
                                    }

                                    Ok(None) => {
                                        if response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).is_err() {
                                            log::debug!("Merkle root relayer {}: HTTP client disconnected before response", self.options.relayer_id);
                                        }
                                    }
                                    Err(err) => {
                                        if response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).is_err() {
                                            log::debug!("Merkle root relayer {}: HTTP client disconnected before response", self.options.relayer_id);
                                        }
                                        return Err(err);
                                    }
                                }
                            }
                        }
                    }


                    None => {
                        log::error!(
                            "Merkle root relayer {}: failed to receive HTTP request",
                            self.options.relayer_id
                        );
                        return Ok(false);
                    }
                }
            }

            block = blocks_rx.recv() => {
                match block {
                    Ok(block) => {
                        let force = ForceGeneration::No;
                        let batch = Batch::Yes;


                        if let Some(bridging_payment_address) = self.options.bridging_payment_address {
                            for (pblock, _) in storage::priority_bridging_paid(&block, bridging_payment_address) {
                                let pblock = self.api_provider.client().get_block_at(pblock).await?;
                                let pblock = GearBlock::from_subxt_block(&client, pblock).await?;
                                log::info!("Merkle root relayer {}: priority bridging requested at block #{}, generating proof for merkle-root at block #{}", self.options.relayer_id, block.number(), pblock.number());

                                self.try_proof_merkle_root(prover, authority_set_sync, pblock, ProofTarget::SignedAnchor, Batch::Yes, Priority::Yes, ForceGeneration::Yes).await?;
                            }
                        }

                        self.try_proof_merkle_root(prover, authority_set_sync, block, ProofTarget::SignedAnchor, batch, Priority::No, force,).await?;
                    }

                    Err(RecvError::Lagged(n)) => {
                        log::warn!(
                            "Merkle root relayer {} lagged behind {n} blocks",
                            self.options.relayer_id
                        );
                        return Ok(true);
                    }

                    Err(RecvError::Closed) => {
                        log::warn!(
                            "Merkle root relayer {}: block listener connection closed, exiting",
                            self.options.relayer_id
                        );
                        return Ok(false);
                    }
                }
            }

            response = prover.recv() => {
                let Some(response) = response else {
                    log::warn!(
                        "Merkle root relayer {}: finality prover connection closed, exiting",
                        self.options.relayer_id
                    );
                    return Ok(false);
                };

                match response {
                    prover::Response::Single {
                        block_number,
                        merkle_root,
                        proof,
                    } => {
                        log::info!(
                            "Merkle root relayer {}: finality proof for block #{block_number} with merkle root {merkle_root} received",
                            self.options.relayer_id
                        );

                        let root_key = (block_number, merkle_root);
                        let local_proof_only = is_local_proof_only(&self.roots, root_key);

                        let merkle_root_entry = self
                            .roots
                            .get_mut(&root_key)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Proven merkle root {merkle_root} for block #{block_number} not found in storage"
                                )
                            })?;
                        merkle_root_entry.single_proof_in_flight = false;
                        merkle_root_entry.proof = Some(proof.clone());
                        if !local_proof_only {
                            merkle_root_entry.status = MerkleRootStatus::SubmitProof;
                        }
                        for rpc in merkle_root_entry.http_requests.drain(..) {
                            if rpc
                                .send(MerkleRootsResponse::MerkleRootProof {
                                    proof: proof.proof.clone(),
                                    proof_block_number: proof.block_number,
                                    block_number,
                                    block_hash: merkle_root_entry.block_hash,
                                    merkle_root,
                                })
                                .is_err()
                            {
                                log::error!(
                                    "Merkle root relayer {}: RPC response send failed",
                                    self.options.relayer_id
                                );
                            }
                        }
                        if !local_proof_only {
                            self.storage.save(&self.roots).await?;
                            self.queue_submission(root_key);
                        }
                    }
                    prover::Response::Batched {
                        block_number,
                        merkle_root,
                        proof,
                        batch_roots,
                    } => {
                        log::info!("Merkle root relayer {}: finality proof for block #{block_number} with merkle root {merkle_root} received (will apply to {} blocks)", self.options.relayer_id, batch_roots.len());

                        if let Some(merkle_root_entry) =
                            self.roots.get_mut(&(block_number, merkle_root))
                        {
                            merkle_root_entry.stage_batched_submission(proof.clone(), batch_roots);
                        } else {
                            return Err(anyhow::anyhow!(
                                "Selected batched merkle root {merkle_root} for block #{block_number} not found in storage"
                            ));
                        }
                        self.storage.save(&self.roots).await?;
                        self.queue_submission((block_number, merkle_root));
                    }
                }
            }



            response = authority_set_sync.recv() => {
                let Some(response) = response else {
                    log::warn!(
                        "Merkle root relayer {}: authority set sync connection closed, exiting",
                        self.options.relayer_id
                    );
                    return Ok(false);
                };

                match response {
                    authority_set_sync::Response::AuthoritySetSynced(id, block) => {
                        self.storage.authority_set_processed(block).await;

                        let queued_blocks = self
                            .waiting_for_authority_set_sync
                            .remove(&id)
                            .map_or(0, |blocks| blocks.len());
                        let inner_proof = self
                            .storage
                            .proofs
                            .get_proof_for_authority_set_id(id)
                            .await?;
                        let pending = self
                            .roots
                            .iter()
                            .filter_map(|(&(block_number, merkle_root), root)| {
                                if matches!(
                                    root.status,
                                    MerkleRootStatus::WaitForAuthoritySetSync(waiting_id, _)
                                        if waiting_id == id
                                ) {
                                    Some((
                                        block_number,
                                        root.block_hash,
                                        merkle_root,
                                        root.queue_id,
                                        root.block_inclusion_proof.clone(),
                                        root.batch,
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();

                        log::info!(
                            "Merkle root relayer {}: authority set #{id} is synced, releasing {} roots from {} queued blocks",
                            self.options.relayer_id,
                            pending.len(),
                            queued_blocks
                        );
                        for (
                            block_number,
                            block_hash,
                            merkle_root,
                            queue_id,
                            block_inclusion_proof,
                            batch,
                        ) in pending
                        {
                            let root = self
                                .roots
                                .get_mut(&(block_number, merkle_root))
                                .expect("pending root was collected from storage");
                            root.status = MerkleRootStatus::GenerateProof;
                            root.single_proof_in_flight = !batch;
                            if !prover.prove(
                                block_number,
                                block_hash,
                                merkle_root,
                                inner_proof.clone(),
                                queue_id,
                                batch,
                                block_inclusion_proof,
                            ) {
                                return Err(anyhow::anyhow!(
                                    "Merkle root relayer {}: prover connection closed while releasing authority set #{id}",
                                    self.options.relayer_id
                                ));
                            }
                        }
                        self.storage.save(&self.roots).await?;

                        if let CriticalThreshold::AuthoritySetChange = self.options.critical_threshold {
                            let block_hash = client.search_for_authority_set_block(id).await?;
                            let block = client.get_block_at(block_hash).await?;
                            let block = GearBlock::from_subxt_block(&client, block).await?;
                            log::info!("Merkle root relayer {}: critical threshold is set to AuthoritySetChange, forcing proof at block #{}", self.options.relayer_id, block.number());
                            self.try_proof_merkle_root(prover, authority_set_sync, block, ProofTarget::SignedAnchor, Batch::No, Priority::No, ForceGeneration::Yes).await?;

                        }
                    }
                }
            }

            continuation = async { self.pending_continuations.pop_front() },
                if !self.pending_continuations.is_empty() => {
                let Some((anchor_key, block_number)) = continuation else {
                    unreachable!("continuation branch requires queued work");
                };
                let block_hash = client.block_number_to_hash(block_number).await?;
                let block = client.get_block_at(block_hash).await?;
                let block = GearBlock::from_subxt_block(&client, block).await?;
                self.try_proof_merkle_root(
                    prover,
                    authority_set_sync,
                    block,
                    ProofTarget::SignedAnchor,
                    Batch::No,
                    Priority::No,
                    ForceGeneration::Yes,
                )
                .await?;

                if let Some(root) = self.roots.get_mut(&anchor_key) {
                    if let Some(index) = root
                        .continuation_source_blocks
                        .iter()
                        .position(|block| *block == block_number)
                    {
                        root.continuation_source_blocks.remove(index);
                    }
                }
                self.storage.save(&self.roots).await?;
                tokio::task::yield_now().await;
            }

            response = submitter.recv() => {
                let Some(response) = response else {
                    log::warn!(
                        "Merkle root relayer {}: proof submitter connection closed, exiting",
                        self.options.relayer_id
                    );
                    return Ok(false);
                };

                let continuations = self.finalize_merkle_root(response).await?;
                self.pending_continuations.extend(continuations);
            }
        }
        Ok(true)
    }

    async fn finalize_merkle_root(
        &mut self,
        response: submitter::Response,
    ) -> anyhow::Result<Vec<PendingContinuation>> {
        if let Some(era) = response.era {
            log::info!(
                "Merkle root relayer {}: era #{} merkle root {} for block #{} is finalized with status: {:?}",
                self.options.relayer_id,
                era,
                response.merkle_root,
                response.merkle_root_block,
                response.status,
            );
        }
        let root_key = (response.merkle_root_block, response.merkle_root);
        let covered_roots = self
            .roots
            .get(&root_key)
            .map(|root| root.covered_roots.clone())
            .unwrap_or_default();
        let is_submitted = matches!(&response.status, submitter::ResponseStatus::Submitted);
        let mut submission_failure = None;

        let confirmation_timestamp_ms = if is_submitted {
            match self
                .roots
                .get(&root_key)
                .filter(|root| {
                    self.last_confirmed_block
                        .is_none_or(|last| root.block_number >= last)
                })
                .map(|root| root.block_hash)
            {
                Some(block_hash) => {
                    match self.api_provider.client().fetch_timestamp(block_hash).await {
                        Ok(timestamp) => Some(timestamp),
                        Err(err) => {
                            log::warn!(
                                "Merkle root relayer {}: failed to refresh confirmed block timestamp: {err}",
                                self.options.relayer_id
                            );
                            None
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        };

        if let Some(merkle_root) = self.roots.get_mut(&root_key) {
            match response.status {
                submitter::ResponseStatus::Submitted => {
                    let proof = merkle_root
                        .proof
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Finalized root has no proof"))?;
                    if proof.block_number != merkle_root.block_number
                        || H256::from(proof.merkle_root) != response.merkle_root
                    {
                        return Err(anyhow::anyhow!(
                            "Finalized proof metadata does not match merkle root {} at block #{}",
                            response.merkle_root,
                            response.merkle_root_block
                        ));
                    }

                    if self
                        .last_confirmed_block
                        .is_none_or(|last| merkle_root.block_number >= last)
                    {
                        self.last_confirmed_block = Some(
                            self.last_confirmed_block
                                .unwrap_or(0)
                                .max(merkle_root.block_number),
                        );
                        self.last_confirmed_timestamp_ms = confirmation_timestamp_ms;
                    }
                    merkle_root.status = MerkleRootStatus::Finalized;
                    log::info!(
                        "Merkle root relayer {}: merkle root {} for block #{} is finalized",
                        self.options.relayer_id,
                        response.merkle_root,
                        response.merkle_root_block
                    );
                    for req in merkle_root.http_requests.drain(..) {
                        if req
                            .send(MerkleRootsResponse::MerkleRootProof {
                                proof: proof.proof.clone(),
                                proof_block_number: proof.block_number,
                                block_number: merkle_root.block_number,
                                block_hash: merkle_root.block_hash,
                                merkle_root: response.merkle_root,
                            })
                            .is_err()
                        {
                            log::debug!(
                                "Merkle root relayer {}: HTTP client disconnected before response",
                                self.options.relayer_id
                            );
                        }
                    }
                }

                submitter::ResponseStatus::Failed(err) => {
                    merkle_root.status = MerkleRootStatus::Failed(err.to_string());
                    submission_failure = Some(err.clone());
                    log::error!(
                        "Merkle root relayer {}: failed to finalize merkle root {} for block #{}: {}",
                        self.options.relayer_id,
                        response.merkle_root,
                        response.merkle_root_block,
                        err
                    );
                    for req in merkle_root.http_requests.drain(..) {
                        if req
                            .send(MerkleRootsResponse::Failed {
                                message: err.clone(),
                            })
                            .is_err()
                        {
                            log::debug!(
                                "Merkle root relayer {}: HTTP client disconnected before response",
                                self.options.relayer_id
                            );
                        }
                    }
                }
            }
        } else {
            return Err(anyhow::anyhow!(
                "Merkle root {} for block #{} not found in storage while finalizing",
                response.merkle_root,
                response.merkle_root_block
            ));
        }

        let continuation_blocks = if is_submitted {
            let (covered_source_blocks, continuation_blocks) =
                finalize_confirmed_roots(&mut self.roots, root_key, &covered_roots);
            for block_number in covered_source_blocks {
                self.storage.merkle_root_processed(block_number).await;
            }
            continuation_blocks
        } else {
            Vec::new()
        };
        self.storage.save(&self.roots).await?;
        if let Some(err) = submission_failure {
            return Err(anyhow::anyhow!(
                "Merkle root submission failed for block #{}: {err}",
                response.merkle_root_block
            ));
        }
        Ok(continuation_blocks)
    }

    /// Attempt to create proof for merkle root of `block`. If authority set that signed `block`
    /// is not yet proven, proof generation will be delayed until authority set is synced.
    #[allow(clippy::too_many_arguments)]
    async fn try_proof_merkle_root(
        &mut self,
        prover: &mut FinalityProverIo,
        authority_set_sync: &mut AuthoritySetSyncIo,
        block: GearBlock,
        proof_target: ProofTarget,
        batch: Batch,
        priority: Priority,
        force_generation: ForceGeneration,
    ) -> anyhow::Result<Option<(u64, H256)>> {
        let api = self.api_provider.client();

        let (queue_id, merkle_root) = if force_generation == ForceGeneration::Yes {
            api.fetch_authenticated_queue_merkle_root(block.hash())
                .await?
        } else {
            let event_state = match storage::queue_merkle_root_changed(&block) {
                Some(merkle_root) => merkle_root,
                None => {
                    log::trace!(
                        "Skipping block #{} as there are no new messages",
                        block.number()
                    );
                    return Ok(None);
                }
            };
            let authenticated_state = api
                .fetch_authenticated_queue_merkle_root(block.hash())
                .await?;
            if event_state != authenticated_state {
                return Err(anyhow::anyhow!(
                    "Queue event at block #{} does not match authenticated Vara state",
                    block.number()
                ));
            }
            event_state
        };

        // finality proof might be available already which happens in the case of
        // merkle roots being inserted there before authority set is synced. Otherwise
        // immediately fetch finality proof.

        let mut block_inclusion_proof = match self
            .roots
            .get(&(block.number(), merkle_root))
            .map(|root| root.block_inclusion_proof.clone())
        {
            Some(proof) => proof,
            None => match self.storage.blocks.read().await.get(&block.number()) {
                Some(block) => block.inclusion_proof.clone(),
                None => block.inclusion_proof(&api).await?,
            },
        };

        let nonces = storage::message_queued_events_of(&block).collect::<Vec<_>>();
        let source_block_number = block.number();
        let source_block_hash = block.hash();
        let source_queue_id = queue_id;
        let source_merkle_root = merkle_root;
        let source_authority_set_id = api
            .fetch_authenticated_signed_by_authority_set_id(source_block_hash)
            .await?;

        let (
            block_number,
            block_hash,
            queue_id,
            merkle_root,
            proof_authority_set_id,
            source_covered,
        ) = match proof_target {
            ProofTarget::Exact => (
                source_block_number,
                source_block_hash,
                source_queue_id,
                source_merkle_root,
                block_inclusion_proof.required_authority_set_id,
                true,
            ),
            ProofTarget::SignedAnchor | ProofTarget::Recovery => {
                let signed_target_block_number = block_inclusion_proof.block_number;
                let contract_limit = contract_anchor_limit(
                    self.last_confirmed_block,
                    self.max_block_distance,
                    signed_target_block_number,
                );
                if contract_limit < signed_target_block_number {
                    let anchor = self.signed_block_at_or_before(contract_limit).await?;
                    block_inclusion_proof = anchor.inclusion_proof(&api).await?;
                    let target_block_number = block_inclusion_proof.block_number;
                    let target_block_hash = block_inclusion_proof.block_hash;
                    let target_authority_set_id = block_inclusion_proof.required_authority_set_id;
                    let (target_queue_id, target_merkle_root) = api
                        .fetch_authenticated_queue_merkle_root(target_block_hash)
                        .await?;
                    if self
                        .last_confirmed_block
                        .is_some_and(|confirmed| target_block_number <= confirmed)
                    {
                        return Err(anyhow::anyhow!(
                            "Contract-gap anchor #{target_block_number} does not advance confirmed block"
                        ));
                    }
                    log::warn!(
                        "Merkle root relayer {}: signed target #{} exceeds contract limit #{}; scheduling intermediate anchor #{}",
                        self.options.relayer_id,
                        signed_target_block_number,
                        contract_limit,
                        target_block_number,
                    );
                    (
                        target_block_number,
                        target_block_hash,
                        target_queue_id,
                        target_merkle_root,
                        target_authority_set_id,
                        false,
                    )
                } else {
                    let target_block_number = block_inclusion_proof.block_number;
                    let target_block_hash = block_inclusion_proof.block_hash;
                    let target_authority_set_id = block_inclusion_proof.required_authority_set_id;
                    let (target_queue_id, target_merkle_root) = api
                        .fetch_authenticated_queue_merkle_root(target_block_hash)
                        .await?;

                    if anchor_covers_source(
                        source_block_number,
                        target_block_number,
                        source_authority_set_id,
                        target_authority_set_id,
                        source_queue_id,
                        target_queue_id,
                    ) {
                        log::debug!(
                            "Merkle root relayer {}: rebasing source block #{} queue #{} onto signed anchor block #{} queue #{}",
                            self.options.relayer_id,
                            source_block_number,
                            source_queue_id,
                            target_block_number,
                            target_queue_id,
                        );
                        (
                            target_block_number,
                            target_block_hash,
                            target_queue_id,
                            target_merkle_root,
                            target_authority_set_id,
                            true,
                        )
                    } else {
                        log::warn!(
                            "Merkle root relayer {}: signed anchor mismatch for source block #{} (authority set #{}, queue #{}): anchor block #{} has authority set #{}, queue #{}; retaining exact source target",
                            self.options.relayer_id,
                            source_block_number,
                            source_authority_set_id,
                            source_queue_id,
                            target_block_number,
                            target_authority_set_id,
                            target_queue_id,
                        );
                        (
                            source_block_number,
                            source_block_hash,
                            source_queue_id,
                            source_merkle_root,
                            block_inclusion_proof.required_authority_set_id,
                            true,
                        )
                    }
                }
            }
        };
        let proof_target_block_hash = block_inclusion_proof.block_hash;

        let selected_key = (block_number, merkle_root);
        let selected_state = self.roots.get(&selected_key).and_then(|root| {
            if reusable_finalized_root(
                root,
                self.last_confirmed_block,
                block_number,
                merkle_root,
                matches!(proof_target, ProofTarget::Recovery),
            ) {
                Some(true)
            } else if matches!(
                root.status,
                MerkleRootStatus::WaitForAuthoritySetSync(_, _)
                    | MerkleRootStatus::GenerateProof
                    | MerkleRootStatus::SubmitProof
                    | MerkleRootStatus::Finalized
            ) {
                Some(false)
            } else {
                None
            }
        });

        if let Some(is_finalized) = selected_state {
            if is_finalized {
                if source_covered {
                    self.storage
                        .merkle_root_processed(source_block_number)
                        .await;
                    self.storage.save(&self.roots).await?;
                }
                return Ok(Some((queue_id, merkle_root)));
            }

            let mut added_nonces = 0;
            let root = self
                .roots
                .get_mut(&selected_key)
                .expect("selected work was read from storage");
            if source_covered {
                if !root.covered_source_blocks.contains(&source_block_number) {
                    root.covered_source_blocks.push(source_block_number);
                }
                for nonce in &nonces {
                    if !root.message_nonces.contains(nonce) {
                        root.message_nonces.push(*nonce);
                        added_nonces += 1;
                    }
                }
            } else if !root
                .continuation_source_blocks
                .contains(&source_block_number)
            {
                root.continuation_source_blocks.push(source_block_number);
            }
            let should_schedule_single = matches!(batch, Batch::No)
                && if matches!(proof_target, ProofTarget::Recovery) {
                    root.promote_to_recovery_proof()
                } else {
                    root.promote_to_single_proof()
                };

            if let Some(index) = self.merkle_root_batch.iter().position(|pending| {
                pending.block_number == block_number && pending.merkle_root == merkle_root
            }) {
                self.merkle_root_batch[index].nonces_count += added_nonces;
                if matches!(priority, Priority::Yes) {
                    self.merkle_root_batch[index].priority = true;
                }
                if matches!(batch, Batch::No) {
                    let pending = self.merkle_root_batch.remove(index);
                    if self.merkle_root_batch.is_empty() {
                        self.first_pending_timestamp.take();
                    }
                    let inclusion_proof = self
                        .roots
                        .get(&selected_key)
                        .expect("selected work was read from storage")
                        .block_inclusion_proof
                        .clone();
                    if !prover.prove(
                        pending.block_number,
                        pending.block_hash,
                        pending.merkle_root,
                        pending.inner_proof,
                        pending.queue_id,
                        false,
                        inclusion_proof,
                    ) {
                        return Err(anyhow::anyhow!("Prover connection closed"));
                    }
                }
            } else if should_schedule_single {
                let inner_proof = self
                    .storage
                    .proofs
                    .get_proof_for_authority_set_id(proof_authority_set_id)
                    .await?;
                let inclusion_proof = self
                    .roots
                    .get(&selected_key)
                    .expect("selected work was read from storage")
                    .block_inclusion_proof
                    .clone();
                if !prover.prove(
                    block_number,
                    block_hash,
                    merkle_root,
                    inner_proof,
                    queue_id,
                    false,
                    inclusion_proof,
                ) {
                    return Err(anyhow::anyhow!("Prover connection closed"));
                }
            }
            self.storage.save(&self.roots).await?;
            return Ok(Some((queue_id, merkle_root)));
        }

        let covered_source_blocks = source_covered
            .then_some(source_block_number)
            .into_iter()
            .collect::<Vec<_>>();
        let continuation_source_blocks = (!source_covered)
            .then_some(source_block_number)
            .into_iter()
            .collect::<Vec<_>>();
        let nonces = if source_covered { nonces } else { Vec::new() };

        self.roots.remove(&selected_key);

        match self
            .storage
            .proofs
            .get_proof_for_authority_set_id(proof_authority_set_id)
            .await
        {
            Ok(inner_proof) => {
                let nonces_count = nonces.len();
                self.roots
                    .entry((block_number, merkle_root))
                    .or_insert(MerkleRoot {
                        queue_id,

                        block_number,
                        block_hash,
                        status: MerkleRootStatus::GenerateProof,
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
                        covered_roots: Vec::new(),
                        covered_source_blocks: covered_source_blocks.clone(),
                        continuation_source_blocks: continuation_source_blocks.clone(),
                        batch: matches!(batch, Batch::Yes),
                        single_proof_in_flight: matches!(batch, Batch::No),
                        block_inclusion_proof: block_inclusion_proof.clone(),
                    });
                if matches!(batch, Batch::Yes) {
                    let now = Instant::now();

                    if self.merkle_root_batch.is_empty() {
                        self.first_pending_timestamp = Some(now);
                    }

                    log::info!("Merkle root relayer {}: merkle-root #{merkle_root} at block #{block_number} with queue #{queue_id} is enqueued for batch processing", self.options.relayer_id);

                    self.queued_root_timestamps.push_back(now);
                    self.merkle_root_batch.push(PendingMerkleRoot {
                        block_hash,
                        block_number,
                        merkle_root,
                        inner_proof,
                        nonces_count,
                        queue_id,
                        priority: matches!(priority, Priority::Yes),
                    });
                    self.storage.save(&self.roots).await?;
                    return Ok(Some((queue_id, merkle_root)));
                }
                log::info!("Merkle root relayer {}: proof for authority set #{proof_authority_set_id} is found, generating proof for merkle-root {merkle_root} at block #{block_number} with queue #{queue_id}", self.options.relayer_id);
                if !prover.prove(
                    block_number,
                    block_hash,
                    merkle_root,
                    inner_proof,
                    queue_id,
                    /* non batching request: should be processed separately */
                    false,
                    block_inclusion_proof,
                ) {
                    log::error!(
                        "Merkle root relayer {}: prover connection closed, exiting...",
                        self.options.relayer_id
                    );
                    return Err(anyhow::anyhow!("Prover connection closed"));
                }
            }

            Err(ProofStorageError::NotInitialized) | Err(ProofStorageError::NotFound(_)) => {
                log::info!(
                    "Merkle root relayer {}: delaying proof generation for merkle root {} at block #{} until authority set #{} is synced",
                    self.options.relayer_id,
                    merkle_root,
                    block_number,
                    proof_authority_set_id,
                );
                self.roots
                    .entry((block_number, merkle_root))
                    .or_insert(MerkleRoot {
                        queue_id,
                        block_number,
                        block_hash,
                        status: MerkleRootStatus::WaitForAuthoritySetSync(
                            proof_authority_set_id,
                            block_number,
                        ),
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
                        covered_roots: Vec::new(),
                        covered_source_blocks,
                        continuation_source_blocks,
                        batch: matches!(batch, Batch::Yes),
                        single_proof_in_flight: false,
                        block_inclusion_proof,
                    });

                // Enqueue an authority set sync for this id whenever the proof is missing.
                // `or_insert_with` de-duplicates per id, so this never spams the runner.
                // We send unconditionally (not only when `signed_by > latest_proven`) so
                // that block-proof requests are self-sufficient even when proof storage is
                // empty (`get_latest_authority_set_id() == None`); in that case the
                // `Request::Initialize`/genesis path is what actually produces the proof,
                // but the `ForceSync` ensures the runner wakes up and emits
                // `Response::AuthoritySetSynced` so waiting blocks / parked HTTP requests
                // are released once the set is available.
                let force_sync = match self.storage.proofs.get_latest_authority_set_id().await {
                    Some(latest) => proof_authority_set_id > latest,
                    None => true,
                };

                let sync_block = if block.hash() == proof_target_block_hash {
                    block
                } else {
                    let target = api.get_block_at(proof_target_block_hash).await?;
                    GearBlock::from_subxt_block(&api, target).await?
                };
                let waiting = self
                    .waiting_for_authority_set_sync
                    .entry(proof_authority_set_id)
                    .or_default();
                if waiting.is_empty() && force_sync && !authority_set_sync.send(sync_block.clone())
                {
                    return Err(anyhow::anyhow!(
                        "Merkle root relayer {}: authority set sync connection closed",
                        self.options.relayer_id
                    ));
                }
                waiting.push(sync_block);
            }

            Err(err) => {
                log::error!(
                    "Merkle root relayer {}: failed to get proof for authority set id {proof_authority_set_id}: {err}",
                    self.options.relayer_id
                );
                self.storage.save(&self.roots).await?;
                return Err(err.into());
            }
        }

        // Source blocks stay replayable until a selected anchor confirms on Ethereum.
        self.storage.save(&self.roots).await?;

        Ok(Some((queue_id, merkle_root)))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MerkleRoot {
    pub block_number: u32,
    pub block_hash: H256,
    pub queue_id: u64,
    pub message_nonces: Vec<U256>,
    #[serde(skip)]
    pub http_requests: Vec<tokio::sync::oneshot::Sender<MerkleRootsResponse>>,
    #[serde(default)]
    pub proof: Option<FinalProof>,
    #[serde(default)]
    pub covered_roots: Vec<(u32, H256)>,
    #[serde(default)]
    pub covered_source_blocks: Vec<u32>,
    #[serde(default = "default_batch")]
    pub batch: bool,
    #[serde(skip)]
    pub single_proof_in_flight: bool,
    #[serde(default)]
    pub continuation_source_blocks: Vec<u32>,
    pub status: MerkleRootStatus,
    pub block_inclusion_proof: RawBlockInclusionProof,
}

impl Clone for MerkleRoot {
    fn clone(&self) -> Self {
        Self {
            block_number: self.block_number,
            block_hash: self.block_hash,
            queue_id: self.queue_id,
            message_nonces: self.message_nonces.clone(),
            http_requests: Vec::new(),
            proof: self.proof.clone(),
            covered_roots: self.covered_roots.clone(),
            batch: self.batch,
            single_proof_in_flight: self.single_proof_in_flight,
            covered_source_blocks: self.covered_source_blocks.clone(),
            continuation_source_blocks: self.continuation_source_blocks.clone(),
            status: self.status.clone(),
            block_inclusion_proof: self.block_inclusion_proof.clone(),
        }
    }
}

fn default_batch() -> bool {
    true
}

impl MerkleRoot {
    fn stage_batched_submission(&mut self, proof: FinalProof, covered_roots: Vec<(u32, H256)>) {
        self.covered_roots = covered_roots;
        self.status = MerkleRootStatus::SubmitProof;
        self.proof = Some(proof);
    }

    fn promote_to_single_proof(&mut self) -> bool {
        self.batch = false;
        if matches!(
            self.status,
            MerkleRootStatus::GenerateProof | MerkleRootStatus::Finalized
        ) && !self.single_proof_in_flight
        {
            self.single_proof_in_flight = true;
            true
        } else {
            false
        }
    }

    fn promote_to_recovery_proof(&mut self) -> bool {
        if matches!(self.status, MerkleRootStatus::Finalized) {
            self.status = MerkleRootStatus::GenerateProof;
        }
        self.promote_to_single_proof()
    }
}

fn recover_finalized_continuations(
    roots: &HashMap<(u32, H256), MerkleRoot>,
) -> Vec<PendingContinuation> {
    let mut continuations = roots
        .iter()
        .filter(|(_, root)| matches!(root.status, MerkleRootStatus::Finalized))
        .flat_map(|(key, root)| {
            root.continuation_source_blocks
                .iter()
                .map(|block| (*key, *block))
        })
        .collect::<Vec<_>>();
    continuations.sort_unstable();
    continuations.dedup();
    continuations
}

fn reusable_finalized_root(
    root: &MerkleRoot,
    last_confirmed_block: Option<u32>,
    block_number: u32,
    merkle_root: H256,
    force_recovery: bool,
) -> bool {
    !force_recovery
        && matches!(root.status, MerkleRootStatus::Finalized)
        && last_confirmed_block.is_some_and(|confirmed| block_number <= confirmed)
        && root.proof.as_ref().is_some_and(|proof| {
            proof.block_number == block_number && H256::from(proof.merkle_root) == merkle_root
        })
}
fn is_local_proof_only(roots: &HashMap<(u32, H256), MerkleRoot>, root_key: (u32, H256)) -> bool {
    roots
        .get(&root_key)
        .is_some_and(|root| matches!(root.status, MerkleRootStatus::Finalized))
        || roots.iter().any(|(anchor_key, anchor)| {
            *anchor_key != root_key
                && matches!(
                    anchor.status,
                    MerkleRootStatus::SubmitProof | MerkleRootStatus::Finalized
                )
                && anchor.covered_roots.contains(&root_key)
        })
}

fn finalize_confirmed_roots(
    roots: &mut HashMap<(u32, H256), MerkleRoot>,
    confirmed_root: (u32, H256),
    covered_roots: &[(u32, H256)],
) -> (Vec<u32>, Vec<PendingContinuation>) {
    let mut covered_source_blocks = Vec::new();
    let mut continuations = Vec::new();

    for key in std::iter::once(&confirmed_root).chain(covered_roots) {
        if let Some(root) = roots.get_mut(key) {
            root.status = MerkleRootStatus::Finalized;
            covered_source_blocks.extend_from_slice(&root.covered_source_blocks);
            continuations.extend(
                root.continuation_source_blocks
                    .iter()
                    .map(|block| (*key, *block)),
            );
        }
    }

    covered_source_blocks.sort_unstable();
    covered_source_blocks.dedup();
    continuations.sort_unstable();
    continuations.dedup();
    (covered_source_blocks, continuations)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MerkleRootStatus {
    WaitForAuthoritySetSync(u64, u32),
    GenerateProof,
    SubmitProof,
    Finalized,
    Failed(String),
}

#[derive(Clone)]
pub struct MerkleRootRelayerOptions {
    pub relayer_id: String,
    pub spike_config: SpikeConfig,
    pub check_interval: Duration,
    pub save_interval: Duration,
    pub genesis_config: GenesisConfig,
    pub last_sealed: Option<u64>,
    pub confirmations: u64,
    pub count_thread: Option<usize>,
    pub bridging_payment_address: Option<H256>,
    /// Condition on which we force merkle-root proof generation.
    pub critical_threshold: CriticalThreshold,
    /// Startup sync strategy for initial catch-up.
    pub startup_sync_strategy: StartupSyncStrategy,
    pub gnark_data_path: PathBuf,
    /// Relayer priority used by shared workers when multiple relayers run in one process.
    pub priority: i64,
    /// When multiple relayers share a process, authority-set proving is serialized through
    /// this shared worker so only one heavy proving job runs at a time.
    pub shared_authority_set_sync: Option<Arc<authority_set_sync::SharedAuthoritySetSync>>,
}

impl MerkleRootRelayerOptions {
    pub fn from_cli(config: &GearEthCoreArgs) -> anyhow::Result<Self> {
        crate::config::EffectiveConfig::from_cli(config)?
            .relayers
            .into_iter()
            .next()
            .map(|relayer| relayer.options)
            .ok_or_else(|| anyhow::anyhow!("No relayer config found"))
    }
}

#[derive(Copy, Clone)]
pub struct SpikeConfig {
    /// Timeout after which we start generating proof
    /// for batch of requests without priority requests.
    pub timeout: Duration,
    /// Timeout after which we start generating proof
    /// for batch of requests with at least one priority request.
    pub priority_timeout: Duration,
    /// Spike window, used to cutoff old merkle-roots
    pub window: Duration,
    /// Spike threshold: after threshold is reached we enter "spike"
    /// mode where proofs are generated immediately.
    pub threshold: usize,
}

impl Default for SpikeConfig {
    fn default() -> Self {
        Self {
            priority_timeout: Duration::from_secs(5 * 60),
            timeout: Duration::from_secs(30 * 60),
            window: Duration::from_secs(15 * 60),
            threshold: 8,
        }
    }
}

pub struct PendingMerkleRoot {
    pub block_hash: H256,
    pub block_number: u32,
    pub merkle_root: H256,
    pub inner_proof: ProofWithCircuitData,
    /// Number of message nonces that are being bridged to Ethereum.
    ///
    /// Used to check for spike.
    pub nonces_count: usize,
    pub queue_id: u64,
    /// Is this request marked as prioritized. If so,
    /// we will take the batch immediately and process it.
    pub priority: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ProofTarget {
    Exact,
    SignedAnchor,
    Recovery,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Priority {
    Yes,
    No,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ForceGeneration {
    Yes,
    No,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Batch {
    Yes,
    No,
}

fn contract_anchor_limit(
    last_confirmed_block: Option<u32>,
    max_block_distance: Option<u32>,
    candidate: u32,
) -> u32 {
    match (last_confirmed_block, max_block_distance) {
        (Some(last), Some(distance)) => candidate.min(last.saturating_add(distance)),
        _ => candidate,
    }
}

fn recovery_target_limit(
    last_confirmed_block: Option<u32>,
    max_block_distance: Option<u32>,
    latest_block: u32,
) -> Option<u32> {
    let last_confirmed_block = last_confirmed_block?;
    let target =
        contract_anchor_limit(Some(last_confirmed_block), max_block_distance, latest_block);
    (target > last_confirmed_block).then_some(target)
}

fn anchor_covers_source(
    source_block: u32,
    target_block: u32,
    source_authority_set_id: u64,
    target_authority_set_id: u64,
    source_queue_id: u64,
    target_queue_id: u64,
) -> bool {
    target_block >= source_block
        && target_authority_set_id == source_authority_set_id
        && target_queue_id == source_queue_id
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CriticalThreshold {
    Timeout(Duration),
    AuthoritySetChange,
}

fn queue_state_changed(confirmed: (u64, H256), latest: (u64, H256)) -> bool {
    confirmed != latest
}

fn critical_timeout_reached(
    critical_threshold: CriticalThreshold,
    last_confirmed_block: Option<u32>,
    last_confirmed_timestamp_ms: Option<u64>,
    block_timestamp_ms: u64,
) -> Option<(u32, Duration)> {
    let CriticalThreshold::Timeout(threshold) = critical_threshold else {
        return None;
    };
    let last_confirmed_block = last_confirmed_block?;
    let last_confirmed_timestamp_ms = last_confirmed_timestamp_ms?;
    let elapsed_ms = block_timestamp_ms.saturating_sub(last_confirmed_timestamp_ms);
    if u128::from(elapsed_ms) >= threshold.as_millis() {
        Some((last_confirmed_block, threshold))
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupSyncStrategy {
    CriticalThreshold,
    SkipCatchUp,
    Blocks(Vec<u32>),
}

#[cfg(test)]
mod tests {
    use super::{
        anchor_covers_source, contract_anchor_limit, critical_timeout_reached, defer_startup_proof,
        finalize_confirmed_roots, is_local_proof_only, queue_state_changed,
        recover_finalized_continuations, recovery_target_limit, reusable_finalized_root,
        CriticalThreshold, FinalProof, MerkleRoot, MerkleRootStatus, RawBlockInclusionProof, H256,
    };
    use std::{collections::HashMap, time::Duration};

    fn root(block_number: u32, block_hash: H256) -> MerkleRoot {
        MerkleRoot {
            block_number,
            block_hash,
            queue_id: 1,
            message_nonces: Vec::new(),
            http_requests: Vec::new(),
            proof: None,
            covered_roots: Vec::new(),
            covered_source_blocks: Vec::new(),
            continuation_source_blocks: Vec::new(),
            batch: true,
            single_proof_in_flight: false,
            status: MerkleRootStatus::GenerateProof,
            block_inclusion_proof: RawBlockInclusionProof {
                justification_round: 0,
                required_authority_set_id: 1,
                validator_set: Vec::new(),
                block_hash,
                block_number,
                pre_commits: Vec::new(),
            },
        }
    }

    #[test]
    fn critical_timeout_is_reached_at_threshold() {
        let threshold = Duration::from_millis(5);
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::Timeout(threshold),
                Some(10),
                Some(100),
                105,
            ),
            Some((10, threshold))
        );
    }

    #[test]
    fn critical_timeout_is_not_reached_before_threshold() {
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::Timeout(Duration::from_millis(5)),
                Some(10),
                Some(100),
                104,
            ),
            None
        );
    }

    #[test]
    fn critical_timeout_requires_last_confirmed_timestamp() {
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::Timeout(Duration::from_millis(5)),
                Some(10),
                None,
                105,
            ),
            None
        );
    }

    #[test]
    fn authority_set_change_is_not_a_timeout_threshold() {
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::AuthoritySetChange,
                Some(10),
                Some(100),
                105,
            ),
            None
        );
    }

    #[test]
    fn recovery_requires_queue_state_change_since_contract_max_block() {
        let confirmed = (7, H256::repeat_byte(1));
        assert!(!queue_state_changed(confirmed, confirmed));
        assert!(queue_state_changed(confirmed, (7, H256::repeat_byte(2))));
        assert!(queue_state_changed(confirmed, (8, H256::repeat_byte(1))));
    }

    #[test]
    fn recovery_target_uses_latest_inside_contract_window_and_caps_beyond_it() {
        assert_eq!(recovery_target_limit(Some(100), Some(25), 100), None);
        assert_eq!(recovery_target_limit(Some(100), Some(25), 124), Some(124));
        assert_eq!(recovery_target_limit(Some(100), Some(25), 125), Some(125));
        assert_eq!(recovery_target_limit(Some(100), Some(25), 200), Some(125));
    }

    #[test]
    fn timeout_inside_window_schedules_fresh_proof_before_resumed_1034_header_proof() {
        const CONFIRMED: u32 = 35_490_266;
        const OLD_BLOCK: u32 = 35_491_497;
        const OLD_SIGNED_BLOCK: u32 = 35_492_530;
        const LATEST: u32 = 35_509_509;
        const MAX_DISTANCE: u32 = 57_600;

        let threshold = Duration::from_secs(14 * 60 * 60);
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::Timeout(threshold),
                Some(CONFIRMED),
                Some(0),
                15 * 60 * 60 * 1_000,
            ),
            Some((CONFIRMED, threshold))
        );
        assert!(queue_state_changed(
            (1, H256::repeat_byte(1)),
            (1, H256::repeat_byte(2)),
        ));
        let recovery_target =
            recovery_target_limit(Some(CONFIRMED), Some(MAX_DISTANCE), LATEST).unwrap();
        assert_eq!(recovery_target, LATEST);

        let old_key = (OLD_BLOCK, H256::repeat_byte(3));
        let mut old_root = root(OLD_BLOCK, H256::repeat_byte(4));
        old_root.block_inclusion_proof.block_number = OLD_SIGNED_BLOCK;
        let mut roots = HashMap::from([(old_key, old_root)]);
        let mut deferred = Vec::new();
        defer_startup_proof(&mut roots, &mut deferred, old_key, 1);

        assert!(matches!(
            roots[&old_key].status,
            MerkleRootStatus::GenerateProof
        ));
        assert!(!roots[&old_key].single_proof_in_flight);
        assert_eq!(deferred.len(), 1);

        let old_span = super::prover::proof_span_order_key(
            deferred[0].block_number,
            deferred[0].block_inclusion_proof.block_number,
        );
        let recovery_span = super::prover::proof_span_order_key(recovery_target, recovery_target);
        assert_eq!(old_span.0, 1_033);
        assert!(recovery_span < old_span);
    }

    #[test]
    fn contract_anchor_limit_keeps_candidate_inside_window() {
        assert_eq!(contract_anchor_limit(Some(100), Some(25), 120), 120);
    }

    #[test]
    fn contract_anchor_limit_caps_candidate_and_saturates_distance() {
        assert_eq!(contract_anchor_limit(Some(100), Some(25), 150), 125);
        assert_eq!(
            contract_anchor_limit(Some(u32::MAX - 1), Some(u32::MAX), u32::MAX),
            u32::MAX
        );
    }

    #[test]
    fn anchor_covers_source_requires_matching_authority_set_and_queue() {
        assert!(anchor_covers_source(100, 120, 7, 7, 9, 9));
        assert!(!anchor_covers_source(100, 120, 7, 8, 9, 9));
        assert!(!anchor_covers_source(100, 120, 7, 7, 9, 10));
    }

    #[test]
    fn anchor_covers_source_rejects_older_target_with_matching_metadata() {
        assert!(!anchor_covers_source(100, 99, 7, 7, 9, 9));
    }

    #[test]
    fn confirmed_anchor_finalizes_covered_roots_and_releases_sources() {
        let covered_key = (100, H256::repeat_byte(1));
        let anchor_key = (120, H256::repeat_byte(2));
        let mut covered_root = root(covered_key.0, H256::repeat_byte(3));
        covered_root.covered_source_blocks = vec![90];
        let mut anchor_root = root(anchor_key.0, H256::repeat_byte(4));
        anchor_root.continuation_source_blocks = vec![130];
        let mut roots = HashMap::from([(covered_key, covered_root), (anchor_key, anchor_root)]);
        let exact_root = roots.get_mut(&covered_key).unwrap();
        assert!(exact_root.promote_to_single_proof());
        assert!(!exact_root.promote_to_single_proof());
        assert!(!exact_root.batch);

        let proof = FinalProof {
            proof: vec![1],
            block_number: anchor_key.0,
            merkle_root: *anchor_key.1.as_fixed_bytes(),
        };

        roots
            .get_mut(&anchor_key)
            .unwrap()
            .stage_batched_submission(proof, vec![covered_key]);
        assert!(matches!(
            roots[&covered_key].status,
            MerkleRootStatus::GenerateProof
        ));
        assert!(is_local_proof_only(&roots, covered_key));

        let (covered_sources, continuation_blocks) =
            finalize_confirmed_roots(&mut roots, anchor_key, &[covered_key]);
        assert!(matches!(
            roots[&covered_key].status,
            MerkleRootStatus::Finalized
        ));
        assert!(matches!(
            roots[&anchor_key].status,
            MerkleRootStatus::Finalized
        ));
        assert!(is_local_proof_only(&roots, covered_key));
        let exact_root = roots.get_mut(&covered_key).unwrap();
        exact_root.single_proof_in_flight = false;
        assert!(exact_root.promote_to_single_proof());
        assert!(matches!(exact_root.status, MerkleRootStatus::Finalized));
        assert_eq!(covered_sources, vec![90]);
        assert_eq!(continuation_blocks, vec![(anchor_key, 130)]);
    }

    #[test]
    fn recovery_does_not_reuse_a_locally_finalized_root() {
        let block_number = 120;
        let merkle_root = H256::repeat_byte(2);
        let mut root = root(block_number, H256::repeat_byte(4));
        root.status = MerkleRootStatus::Finalized;
        root.proof = Some(FinalProof {
            proof: vec![1],
            block_number,
            merkle_root: *merkle_root.as_fixed_bytes(),
        });

        assert!(reusable_finalized_root(
            &root,
            Some(block_number),
            block_number,
            merkle_root,
            false,
        ));
        assert!(!reusable_finalized_root(
            &root,
            Some(block_number),
            block_number,
            merkle_root,
            true,
        ));

        assert!(root.promote_to_recovery_proof());
        assert!(matches!(root.status, MerkleRootStatus::GenerateProof));
        assert!(root.single_proof_in_flight);
    }

    #[test]
    fn restart_recovers_only_finalized_continuations() {
        let finalized_key = (120, H256::repeat_byte(2));
        let pending_key = (121, H256::repeat_byte(3));
        let mut finalized = root(finalized_key.0, H256::repeat_byte(4));
        finalized.status = MerkleRootStatus::Finalized;
        finalized.continuation_source_blocks = vec![131, 130];
        let mut pending = root(pending_key.0, H256::repeat_byte(5));
        pending.continuation_source_blocks = vec![132];
        let roots = HashMap::from([(finalized_key, finalized), (pending_key, pending)]);

        assert_eq!(
            recover_finalized_continuations(&roots),
            vec![(finalized_key, 130), (finalized_key, 131)]
        );
    }

    #[test]
    fn timeout_and_contract_limit_require_confirmed_cursor() {
        assert_eq!(
            critical_timeout_reached(
                CriticalThreshold::Timeout(Duration::from_millis(5)),
                None,
                Some(100),
                105,
            ),
            None
        );
        assert_eq!(contract_anchor_limit(None, Some(25), 150), 150);
        assert_eq!(contract_anchor_limit(Some(100), None, 150), 150);
    }
}
