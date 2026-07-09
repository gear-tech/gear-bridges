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
    collections::{BTreeMap, HashMap, VecDeque},
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
        last_submitted_block: IntGauge = IntGauge::new(
            "merkle_root_relayer_last_submitted_block",
            "Block number of the last submitted merkle root"
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

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,

    storage: Arc<MerkleRootStorage>,

    roots: HashMap<(u32, H256), MerkleRoot>,

    /// Set of blocks that are waiting for authority set sync.
    waiting_for_authority_set_sync: BTreeMap<u64, Vec<GearBlock>>,

    last_submitted_block: Option<u32>,
    /// Merkle root hash of the last supervisor-triggered proof request.
    /// Used to avoid re-triggering the same root while it's in-flight.
    /// Keyed on hash only — the same merkle root can appear at different block numbers.
    last_supervisor_trigger: Option<H256>,
    first_pending_timestamp: Option<Instant>,
    queued_root_timestamps: VecDeque<Instant>,
    merkle_root_batch: Vec<PendingMerkleRoot>,

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

            last_submitted_block: None,
            last_supervisor_trigger: None,
            first_pending_timestamp: None,
            queued_root_timestamps: VecDeque::with_capacity(8),
            merkle_root_batch: Vec::with_capacity(8),

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

    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        mut self,
        mut blocks_rx: Receiver<GearBlock>,
        mut submitter: SubmitterIo,
        mut prover: FinalityProverIo,
        mut authority_set_sync: AuthoritySetSyncIo,
        mut http: UnboundedReceiver<MerkleRootsRequest>,

        eth_api: EthApi,
    ) -> anyhow::Result<()> {
        let relayer_id = self.options.relayer_id.clone();
        log::info!("Starting merkle root relayer {relayer_id}");
        let mut roots = match self.storage.load().await {
            Ok(roots) => roots,
            Err(err) => {
                log::error!(
                    "Merkle root relayer {relayer_id}: failed to load merkle roots from storage: {err}"
                );
                Default::default()
            }
        };
        let mut last_sealed = match self.options.last_sealed {
            Some(era) => era,
            None => {
                let block = rpc::retry_gear(
                    &mut self.api_provider,
                    "startup latest finalized block",
                    |api| async move { api.latest_finalized_block().await },
                )
                .await
                .context("Failed to get latest finalized block during startup")?;
                rpc::retry_gear(
                    &mut self.api_provider,
                    "startup authority set id",
                    move |api| async move { api.authority_set_id(block).await },
                )
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

        for ((block_number, hash), merkle_root) in roots.drain() {
            let block_hash = merkle_root.block_hash;

            let mut reinstate = |status: MerkleRootStatus| {
                self.roots.insert(
                    (block_number, hash),
                    MerkleRoot {
                        queue_id: 0,
                        block_number,
                        block_hash,
                        status,
                        message_nonces: Vec::new(),
                        proof: merkle_root.proof.clone(),
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

                    let block = rpc::retry_gear(
                        &mut self.api_provider,
                        "recovery get_block_at",
                        move |api| async move { api.get_block_at(block_hash).await },
                    )
                    .await?;
                    let client = self.api_provider.client();
                    let block = GearBlock::from_subxt_block(&client, block).await?;

                    match self
                        .storage
                        .proofs
                        .get_proof_for_authority_set_id(*id)
                        .await
                    {
                        Ok(authority_set_proof) => {
                            reinstate(MerkleRootStatus::GenerateProof);

                            if !prover.prove(
                                block_number,
                                block_hash,
                                hash,
                                authority_set_proof,
                                merkle_root.queue_id,
                                true,
                                merkle_root.block_inclusion_proof.clone(),
                            ) {
                                log::error!(
                                    "Merkle root relayer {relayer_id}: prover connection closed, exiting..."
                                );
                                return Err(anyhow::anyhow!(
                                    "Merkle root relayer {relayer_id}: prover connection closed during startup recovery"
                                ));
                            }
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
                                last_sealed = *id - 1;
                            }

                            let force_sync = self
                                .storage
                                .proofs
                                .get_latest_authority_set_id()
                                .await
                                .is_some_and(|latest| *id > latest)
                                && *id > last_sealed;

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

                    log::info!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} is waiting for proof generation"
                    );

                    // if merkle root was saved in `generate proof` phase, it means
                    // that proof for authority set id is already generated and thus should be available in storage.
                    // If it is not found that is a hard error and storage should be fixed.
                    let block_hash_for_authority = block_hash;
                    let signed_by_authority_set_id = rpc::retry_gear(
                        &mut self.api_provider,
                        "recovery signed_by_authority_set_id",
                        move |api| async move {
                            api.signed_by_authority_set_id(block_hash_for_authority).await
                        },
                    )
                    .await?;
                    let inner_proof = self
                        .storage
                        .proofs
                        .get_proof_for_authority_set_id(signed_by_authority_set_id)
                        .await
                        .with_context(|| format!("Proof for authority set #{signed_by_authority_set_id} not found, please clean-up your storage and restart relayer"))?;

                    if !prover.prove(
                        block_number,
                        block_hash,
                        hash,
                        inner_proof,
                        merkle_root.queue_id,
                        true,
                        merkle_root.block_inclusion_proof.clone(),
                    ) {
                        log::error!(
                            "Merkle root relayer {relayer_id}: prover connection closed, exiting..."
                        );
                        return Err(anyhow::anyhow!(
                            "Merkle root relayer {relayer_id}: prover connection closed during startup recovery"
                        ));
                    }
                }

                MerkleRootStatus::SubmitProof => {
                    log::info!(
                        "Merkle root relayer {relayer_id}: merkle root {hash} for block #{block_number} is waiting for proof submission"
                    );
                    let proof = merkle_root
                        .proof
                        .clone()
                        .expect("proof should be available if root is in SubmitProof state; check your storage");

                    reinstate(MerkleRootStatus::SubmitProof);

                    if !submitter.submit_merkle_root(block_number, hash, proof) {
                        log::error!(
                            "Merkle root relayer {relayer_id}: proof submitter connection closed, exiting"
                        );
                        return Err(anyhow::anyhow!(
                            "Merkle root relayer {relayer_id}: proof submitter connection closed during startup recovery"
                        ));
                    }
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

        self.initialize_contract_cursor(&eth_api).await?;
        self.supervise_contract_state(&mut prover, &mut authority_set_sync, &eth_api)
            .await?;
        self.supervisor_interval.tick().await;

        if let Err(err) = self
            .run_inner(
                &mut submitter,
                &mut prover,
                &mut blocks_rx,
                &mut authority_set_sync,
                &mut http,
                &eth_api,
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

    async fn initialize_contract_cursor(&mut self, eth_api: &EthApi) -> anyhow::Result<()> {
        let relayer_id = &self.options.relayer_id;
        let max_block_number = eth_api.max_block_number().await?;
        let max_block_distance = eth_api.max_block_distance().await?;
        let last_block_hash = rpc::retry_gear(
            &mut self.api_provider,
            "initialize_contract_cursor latest_finalized_block",
            |api| async move { api.latest_finalized_block().await },
        )
        .await?;
        let client = self.api_provider.client();
        let last_block = client.block_hash_to_number(last_block_hash).await?;
        let max_block_number_in_storage = self.max_finalized_merkle_root_block();

        log::info!("Merkle root relayer {relayer_id}: latest finalized block is #{last_block}, max block number in Ethereum MessageQueue contract is #{max_block_number} (MAX_BLOCK_DISTANCE={max_block_distance})");
        if let Some(max_stored) = max_block_number_in_storage {
            log::info!(
                "Merkle root relayer {relayer_id}: max finalized merkle root in storage is at block #{max_stored}"
            );
        } else {
            log::info!("Merkle root relayer {relayer_id}: no finalized merkle roots in storage");
        }

        self.last_submitted_block = Some(max_block_number_in_storage.unwrap_or(max_block_number));
        Ok(())
    }

    fn max_finalized_merkle_root_block(&self) -> Option<u32> {
        self.roots
            .values()
            .filter(|root| {
                root.proof.is_some() && matches!(root.status, MerkleRootStatus::Finalized)
            })
            .map(|root| root.block_number)
            .max()
    }

    async fn supervise_contract_state(
        &mut self,
        prover: &mut FinalityProverIo,
        authority_set_sync: &mut AuthoritySetSyncIo,
        eth_api: &EthApi,
    ) -> anyhow::Result<()> {
        let relayer_id = self.options.relayer_id.clone();
        let client = self.api_provider.client();
        let last_block_hash = client.latest_finalized_block().await?;
        let last_block = client.block_hash_to_number(last_block_hash).await?;

        log::info!(
            "Merkle root relayer {relayer_id} supervisor: checking Vara queue state near latest finalized block #{last_block}"
        );

        let block = self.signed_block_after(last_block).await?;
        let block_number = block.number();
        let block_hash = block.hash();
        let (queue_id, merkle_root) = client.fetch_queue_merkle_root(block_hash).await?;

        if merkle_root == H256::zero() {
            log::trace!(
                "Merkle root relayer {relayer_id} supervisor: latest Vara queue root is zero at block #{block_number}, skipping"
            );
            return Ok(());
        }

        let eth_root = eth_api
            .read_chainhead_merkle_root(block_number)
            .await?
            .map(H256::from);
        if eth_root == Some(merkle_root) {
            log::info!(
                "Merkle root relayer {relayer_id} supervisor: Ethereum already has merkle root {merkle_root} for block #{block_number}"
            );
            self.storage
                .submitted_merkle_root(block_number, merkle_root)
                .await;
            return Ok(());
        }

        if self
            .storage
            .is_merkle_root_submitted(block_number, merkle_root)
            .await
        {
            log::info!(
                "Merkle root relayer {relayer_id} supervisor: merkle root {merkle_root} for block #{block_number} was already submitted and is waiting for Ethereum confirmations"
            );
            return Ok(());
        }

        // Two independent triggers — either one is sufficient to schedule proof generation.
        let merkle_hash_trigger = eth_root != Some(merkle_root);
        let critical_threshold_trigger = critical_timeout_reached(
            self.options.critical_threshold,
            self.last_submitted_block,
            block_number,
        )
        .is_some();

        if !merkle_hash_trigger && !critical_threshold_trigger {
            log::debug!(
                "Merkle root relayer {relayer_id} supervisor: Ethereum has merkle root and critical threshold not reached, skipping"
            );
            return Ok(());
        }

        if merkle_hash_trigger {
            if eth_root.is_none() {
                log::info!(
                    "Merkle root relayer {relayer_id} supervisor: [merkle hash trigger] Ethereum has no merkle root for Vara queue root {merkle_root} at block #{block_number}"
                );
            } else {
                log::warn!(
                    "Merkle root relayer {relayer_id} supervisor: [merkle hash trigger] Ethereum has merkle root {:?} but Vara has {merkle_root} at block #{block_number}",
                    eth_root.unwrap()
                );
            }
        }
        if critical_threshold_trigger {
            let (last_submitted, threshold) = critical_timeout_reached(
                self.options.critical_threshold,
                self.last_submitted_block,
                block_number,
            )
            .unwrap();
            log::warn!(
                "Merkle root relayer {relayer_id} supervisor: [critical threshold trigger] last submitted block {last_submitted} is older than block #{block_number} by at least {threshold}"
            );
        }

        // Dedup: skip if this merkle root was already triggered and is in-flight.
        if self.last_supervisor_trigger == Some(merkle_root) {
            if let Some(root) = self.roots.get(&(block_number, merkle_root)) {
                if matches!(root.status, MerkleRootStatus::GenerateProof | MerkleRootStatus::SubmitProof) {
                    log::debug!(
                        "Merkle root relayer {relayer_id} supervisor: merkle root {merkle_root} already in-flight, skipping"
                    );
                    return Ok(());
                }
            }
        }

        self.last_supervisor_trigger = Some(merkle_root);
        self.try_proof_merkle_root(
            prover,
            authority_set_sync,
            block,
            Batch::No,
            Priority::No,
            ForceGeneration::Yes,
        )
        .await?;
        log::info!(
            "Merkle root relayer {relayer_id} supervisor: proof request scheduled for queue #{queue_id}, merkle root {merkle_root} at block #{block_number}"
        );
        Ok(())
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

    async fn run_inner(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,

        http: &mut UnboundedReceiver<MerkleRootsRequest>,
        eth_api: &EthApi,
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

            if let Err(err) = self.storage.save(&self.roots).await {
                log::error!(
                    "Merkle root relayer {}: failed to save block state: {err:?}",
                    self.options.relayer_id
                );
            }

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
                    if rpc::classify_anyhow(&err) == rpc::RetryDecision::Retry {
                        log::warn!(
                            "Merkle root relayer {}: recoverable error, reconnecting and resuming",
                            self.options.relayer_id
                        );
                        self.api_provider.reconnect().await.ok();
                        continue;
                    }
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
        eth_api: &EthApi,
    ) -> anyhow::Result<bool> {
        tokio::select! {
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
                self.metrics.last_submitted_block.set(self.last_submitted_block.unwrap_or(0) as i64);
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
                                "Merkle root relayer {}: prover connection closed, will retry",
                                self.options.relayer_id
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            return Ok(true);
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
                                // filter by `proof.is_some()` since some old storage entries do not contain proofs in them.
                                if let Some((&(_, merkle_root), root)) = self.roots.iter().find(|(_, r)| r.block_number == block_number || r.proof.as_ref().filter(|proof| proof.block_number == block_number).is_some()).filter(|(_, r)| r.proof.is_some()) {
                                    if let MerkleRootStatus::Finalized = root.status {
                                        let proof = root.proof.as_ref().expect("proof availability is checked above");
                                        let Ok(_) = response.send(MerkleRootsResponse::MerkleRootProof {
                                            proof: proof.proof.clone(),
                                            proof_block_number: proof.block_number,
                                            block_number: root.block_number,
                                            block_hash: root.block_hash,
                                            merkle_root,
                                        }) else {
                                            log::error!("Merkle root relayer {}: HTTP response send failed", self.options.relayer_id);
                                            return Ok(false);
                                        };
                                        return Ok(true);
                                    }
                                }

                                // Use the latest GRANDPA-justified block (chain len = 1) for
                                // fast proof generation instead of fetching the specific
                                // requested block. Retry up to 5 times with fresh blocks on failure.
                                const HTTP_PROOF_RETRIES: u32 = 5;
                                let mut http_attempt = 0u32;
                                loop {
                                    let block = match self.signed_block_after(block_number).await {
                                        Ok(b) => b,
                                        Err(err) => {
                                            log::error!("Merkle root relayer {}: HTTP handle failed to find justified block: {err}", self.options.relayer_id);
                                            response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).ok();
                                            return Ok(true);
                                        }
                                    };

                                    match self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::No, Priority::Yes, ForceGeneration::Yes).await {
                                        Ok(Some((_, merkle_root))) => {
                                            if let Some(r) = self.roots.get_mut(&(block_number, merkle_root)) { r.http_requests.push(response) } else {
                                                response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).ok();
                                            }
                                            break;
                                        }

                                        Ok(None) => {
                                            let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                                log::error!("Merkle root relayer {}: HTTP response send failed", self.options.relayer_id);
                                                return Ok(false);
                                            };
                                            break;
                                        }
                                        Err(err) => {
                                            http_attempt += 1;
                                            if http_attempt >= HTTP_PROOF_RETRIES {
                                                log::error!("Merkle root relayer {}: HTTP proof generation failed after {HTTP_PROOF_RETRIES} attempts: {err}", self.options.relayer_id);
                                                response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).ok();
                                                return Ok(true);
                                            }
                                            log::warn!("Merkle root relayer {}: HTTP proof attempt {http_attempt}/{HTTP_PROOF_RETRIES} failed: {err}, retrying with fresh block", self.options.relayer_id);
                                            tokio::time::sleep(Duration::from_secs(2)).await;
                                        }
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
                        let mut force = ForceGeneration::No;
                        let mut batch = Batch::Yes;
                        let number = block.number();
                        if let Some((last_submitted_block, threshold)) =
                            critical_timeout_reached(
                                self.options.critical_threshold,
                                self.last_submitted_block,
                                number,
                            )
                        {
                            log::warn!("Merkle root relayer {}: last submitted block {last_submitted_block} is older than current block number {number} by at least {threshold}, forcing proof generation", self.options.relayer_id);
                            force = ForceGeneration::Yes;
                            batch = Batch::No;
                        }


                        if let Some(bridging_payment_address) = self.options.bridging_payment_address {
                            for (pblock, _) in storage::priority_bridging_paid(&block, bridging_payment_address) {
                                let pblock_hash = rpc::retry_gear(
                                    &mut self.api_provider,
                                    "priority bridging get_block_at",
                                    move |api| async move { api.get_block_at(pblock).await },
                                )
                                .await?;
                                let client = self.api_provider.client();
                                let pblock = GearBlock::from_subxt_block(&client, pblock_hash).await?;
                                log::info!("Merkle root relayer {}: priority bridging requested at block #{}, generating proof for merkle-root at block #{}", self.options.relayer_id, block.number(), pblock.number());

                                self.try_proof_merkle_root(prover, authority_set_sync, pblock, Batch::Yes, Priority::Yes, ForceGeneration::Yes).await?;
                            }
                        }

                        self.try_proof_merkle_root(prover, authority_set_sync, block, batch, Priority::No, force,).await?;
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
                        "Merkle root relayer {}: finality prover connection closed, will retry",
                        self.options.relayer_id
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return Ok(true);
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

                        self.roots.entry((block_number, merkle_root))
                            .and_modify(|merkle_root_entry| {
                                merkle_root_entry.status = MerkleRootStatus::SubmitProof;
                                merkle_root_entry.proof = Some(proof.clone());
                                for rpc in merkle_root_entry.http_requests.drain(..) {
                                    let Ok(_) = rpc.send(MerkleRootsResponse::MerkleRootProof {
                                        proof: proof.proof.clone(),
                                        proof_block_number: proof.block_number,
                                        block_number,
                                        block_hash: merkle_root_entry.block_hash,
                                        merkle_root
                                    }) else {
                                        log::error!("Merkle root relayer {}: RPC response send failed", self.options.relayer_id);
                                        continue;
                                    };
                                }
                            });


                        if !submitter.submit_merkle_root(block_number, merkle_root, proof) {
                            log::warn!(
                                "Merkle root relayer {}: proof submitter connection closed, will retry",
                                self.options.relayer_id
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            return Ok(true);
                        }
                    }

                    prover::Response::Batched {
                        block_number,
                        merkle_root,
                        proof,
                        batch_roots
                    } => {
                        log::info!("Merkle root relayer {}: finality proof for block #{block_number} with merkle root {merkle_root} received (will apply to {} blocks)", self.options.relayer_id, batch_roots.len());

                        for (block_number, merkle_root) in batch_roots {
                            log::debug!("Merkle root relayer {}: merkle-root {merkle_root} finalized as part of batch for block #{block_number}", self.options.relayer_id);
                            self.roots.entry((block_number, merkle_root))
                                .and_modify(|merkle_root_entry| {
                                    merkle_root_entry.status = MerkleRootStatus::Finalized;
                                    merkle_root_entry.proof = Some(proof.clone());
                                    for rpc in merkle_root_entry.http_requests.drain(..) {
                                        let Ok(_) = rpc.send(MerkleRootsResponse::MerkleRootProof {
                                            proof: proof.proof.clone(),
                                            proof_block_number: proof.block_number,
                                            block_number,
                                            block_hash: merkle_root_entry.block_hash,
                                            merkle_root,
                                        }) else {
                                            log::error!("Merkle root relayer {}: RPC response send failed", self.options.relayer_id);
                                            continue;
                                        };
                                        log::info!("Merkle root relayer {}: send HTTP response for merkle root {merkle_root} at block #{block_number}", self.options.relayer_id);
                                    }
                            });

                        }

                        self.roots.entry((block_number, merkle_root))
                            .and_modify(|merkle_root_entry| {
                                merkle_root_entry.status = MerkleRootStatus::SubmitProof;
                                merkle_root_entry.proof = Some(proof.clone());
                                for rpc in merkle_root_entry.http_requests.drain(..) {
                                    let Ok(_) = rpc.send(MerkleRootsResponse::MerkleRootProof {
                                        proof: proof.proof.clone(),
                                        proof_block_number: proof.block_number,
                                        block_number,
                                        block_hash: merkle_root_entry.block_hash,
                                        merkle_root,
                                    }) else {
                                        log::error!("Merkle root relayer {}: RPC response send failed", self.options.relayer_id);
                                        continue;
                                    };
                                }
                            });

                        if !submitter.submit_merkle_root(block_number, merkle_root, proof) {
                            log::warn!(
                                "Merkle root relayer {}: proof submitter connection closed, will retry",
                                self.options.relayer_id
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            return Ok(true);
                        }
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

                        let Some(mut to_submit) = self.waiting_for_authority_set_sync.remove(&id) else {
                            log::warn!("Merkle root relayer {}: no blocks to sync for authority set #{id}", self.options.relayer_id);
                            return Ok(true)
                        };

                        log::info!("Merkle root relayer {}: authority set #{id} is synced, submitting {} blocks", self.options.relayer_id, to_submit.len());
                        while let Some(block) = to_submit.pop() {
                            self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::Yes, Priority::No, ForceGeneration::Yes).await?;
                        }

                        if let CriticalThreshold::AuthoritySetChange = self.options.critical_threshold {
                            let block_hash = rpc::retry_gear(
                                &mut self.api_provider,
                                "authority-set-synced search_for_authority_set_block",
                                move |api| async move { api.search_for_authority_set_block(id).await },
                            )
                            .await?;
                            let block = rpc::retry_gear(
                                &mut self.api_provider,
                                "authority-set-synced get_block_at",
                                move |api| async move { api.get_block_at(block_hash).await },
                            )
                            .await?;
                            let client = self.api_provider.client();
                            let block = GearBlock::from_subxt_block(&client, block).await?;
                            log::info!("Merkle root relayer {}: critical threshold is set to AuthoritySetChange, forcing proof at block #{}", self.options.relayer_id, block.number());
                            self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::No, Priority::No, ForceGeneration::Yes).await?;

                        }
                    }
                }
            }

            response = submitter.recv() => {
                let Some(response) = response else {
                    log::warn!(
                        "Merkle root relayer {}: proof submitter connection closed, will retry",
                        self.options.relayer_id
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return Ok(true);
                };

                self.finalize_merkle_root(response).await?;
            }
        }
        Ok(true)
    }

    async fn finalize_merkle_root(&mut self, response: submitter::Response) -> anyhow::Result<()> {
        // Clear supervisor dedup trigger so the next supervisor tick can re-trigger if needed.
        if self.last_supervisor_trigger == Some(response.merkle_root) {
            self.last_supervisor_trigger = None;
        }

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
        if let Some(merkle_root) = self
            .roots
            .get_mut(&(response.merkle_root_block, response.merkle_root))
        {
            match response.status {
                submitter::ResponseStatus::Submitted => {
                    self.last_submitted_block = match self.last_submitted_block {
                        Some(n) if merkle_root.block_number > n => Some(merkle_root.block_number),
                        _ => Some(merkle_root.block_number),
                    };
                    merkle_root.status = MerkleRootStatus::Finalized;
                    log::info!(
                        "Merkle root relayer {}: merkle root {} for block #{} is finalized",
                        self.options.relayer_id,
                        response.merkle_root,
                        response.merkle_root_block
                    );
                    let proof = merkle_root
                        .proof
                        .as_ref()
                        .expect("proof should be available if root is finalized");
                    for req in merkle_root.http_requests.drain(..) {
                        let Ok(_) = req.send(MerkleRootsResponse::MerkleRootProof {
                            proof: proof.proof.clone(),
                            proof_block_number: proof.block_number,
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            merkle_root: response.merkle_root,
                        }) else {
                            log::error!(
                                "Merkle root relayer {}: HTTP response send failed",
                                self.options.relayer_id
                            );
                            return Err(anyhow::anyhow!("HTTP response send failed"));
                        };
                    }
                }

                submitter::ResponseStatus::Failed(err) => {
                    merkle_root.status = MerkleRootStatus::Failed(err.to_string());
                    log::error!(
                        "Merkle root relayer {}: failed to finalize merkle root {} for block #{}: {}",
                        self.options.relayer_id,
                        response.merkle_root,
                        response.merkle_root_block,
                        err
                    );
                    for req in merkle_root.http_requests.drain(..) {
                        let Ok(_) = req.send(MerkleRootsResponse::Failed {
                            message: err.clone(),
                        }) else {
                            log::error!(
                                "Merkle root relayer {}: HTTP response send failed",
                                self.options.relayer_id
                            );
                            return Err(anyhow::anyhow!("HTTP response send failed"));
                        };
                    }
                }
            }
        } else {
            log::warn!(
                "Merkle root relayer {}: merkle root {} for block #{} not found in storage",
                self.options.relayer_id,
                response.merkle_root,
                response.merkle_root_block
            );
        }

        Ok(())
    }

    /// Attempt to create proof for merkle root of `block`. If authority set that signed `block`
    /// is not yet proven, proof generation will be delayed until authority set is synced.
    #[allow(clippy::too_many_arguments)]
    async fn try_proof_merkle_root(
        &mut self,
        prover: &mut FinalityProverIo,
        authority_set_sync: &mut AuthoritySetSyncIo,
        block: GearBlock,
        batch: Batch,
        priority: Priority,
        force_generation: ForceGeneration,
    ) -> anyhow::Result<Option<(u64, H256)>> {
        let block_hash_for_fetch = block.hash();
        let (queue_id, merkle_root) = if force_generation == ForceGeneration::Yes {
            rpc::retry_gear(
                &mut self.api_provider,
                "fetch_queue_merkle_root",
                move |api| async move { api.fetch_queue_merkle_root(block_hash_for_fetch).await },
            )
            .await?
        } else {
            match storage::queue_merkle_root_changed(&block) {
                Some(merkle_root) => merkle_root,
                None => {
                    log::trace!(
                        "Skipping block #{} as there are no new messages",
                        block.number()
                    );
                    return Ok(None);
                }
            }
        };

        // finality proof might be available already which happens in the case of
        // merkle roots being inserted there before authority set is synced. Otherwise
        // immediately fetch finality proof.

        let block_inclusion_proof = match self
            .roots
            .get(&(block.number(), merkle_root))
            .map(|root| root.block_inclusion_proof.clone())
        {
            Some(proof) => proof,
            None => match self.storage.blocks.read().await.get(&block.number()) {
                Some(block) => block.inclusion_proof.clone(),
                None => {
                let client = self.api_provider.client();
                block.inclusion_proof(&client).await?
            }
            },
        };

        let nonces = storage::message_queued_events_of(&block).collect::<Vec<_>>();

        if self
            .storage
            .is_merkle_root_submitted(block.number(), merkle_root)
            .await
            && force_generation == ForceGeneration::No
        {
            log::debug!(
                "Merkle root relayer {}: skipping merkle root {} for block #{} as there were no new messages",
                self.options.relayer_id,
                merkle_root,
                block.number()
            );
            self.storage.merkle_root_processed(block.number()).await;
            if let Err(err) = self.storage.save(&self.roots).await {
                log::error!(
                    "Merkle root relayer {}: failed to save block storage state: {err:?}",
                    self.options.relayer_id
                );
            }
            return Ok(None);
        }

        let block_hash_for_authority = block.hash();
        let signed_by_authority_set_id = rpc::retry_gear(
            &mut self.api_provider,
            "merkle root signed authority set id",
            move |api| async move {
                api.signed_by_authority_set_id(block_hash_for_authority)
                    .await
            },
        )
        .await?;

        let block_number = block.number();

        match self
            .storage
            .proofs
            .get_proof_for_authority_set_id(signed_by_authority_set_id)
            .await
        {
            Ok(inner_proof) => {
                let block_hash = block.hash();
                let nonces_count = nonces.len();
                self.last_submitted_block = match self.last_submitted_block {
                    Some(n) if block.number() > n => Some(block.number()),
                    _ => Some(block.number()),
                };
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
                    return Ok(Some((queue_id, merkle_root)));
                }
                log::info!("Merkle root relayer {}: proof for authority set #{signed_by_authority_set_id} is found, generating proof for merkle-root {merkle_root} at block #{block_number} with queue #{queue_id}", self.options.relayer_id);
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
                    block.number(),
                    signed_by_authority_set_id,
                );
                self.roots
                    .entry((block_number, merkle_root))
                    .or_insert(MerkleRoot {
                        queue_id,
                        block_number: block.number(),
                        block_hash: block.hash(),
                        status: MerkleRootStatus::WaitForAuthoritySetSync(
                            signed_by_authority_set_id,
                            block.number(),
                        ),
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
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
                    Some(latest) => signed_by_authority_set_id > latest,
                    None => true,
                };

                let waiting = self
                    .waiting_for_authority_set_sync
                    .entry(signed_by_authority_set_id)
                    .or_default();
                if waiting.is_empty() && force_sync && !authority_set_sync.send(block.clone()) {
                    return Err(anyhow::anyhow!(
                        "Merkle root relayer {}: authority set sync connection closed",
                        self.options.relayer_id
                    ));
                }
                waiting.push(block);
            }

            Err(err) => {
                self.roots.insert(
                    (block_number, merkle_root),
                    MerkleRoot {
                        queue_id,

                        block_number: block.number(),
                        block_hash: block.hash(),
                        status: MerkleRootStatus::Failed(err.to_string()),
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
                        block_inclusion_proof,
                    },
                );

                log::error!(
                    "Merkle root relayer {}: failed to get proof for authority set id {signed_by_authority_set_id}: {err}",
                    self.options.relayer_id
                );
                self.storage.merkle_root_processed(block_number).await;
                if let Err(save_err) = self.storage.save(&self.roots).await {
                    log::error!(
                        "Merkle root relayer {}: failed to save block storage state: {save_err:?}",
                        self.options.relayer_id
                    );
                }
                return Err(err.into());
            }
        }

        // Mark root processed only after durable root state exists or the work is queued.
        self.storage.merkle_root_processed(block_number).await;
        if let Err(err) = self.storage.save(&self.roots).await {
            log::error!(
                "Merkle root relayer {}: failed to save block storage state: {err:?}",
                self.options.relayer_id
            );
        }

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
            status: self.status.clone(),
            block_inclusion_proof: self.block_inclusion_proof.clone(),
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CriticalThreshold {
    Timeout(u32),
    AuthoritySetChange,
}

fn critical_timeout_reached(
    critical_threshold: CriticalThreshold,
    last_submitted_block: Option<u32>,
    block_number: u32,
) -> Option<(u32, u32)> {
    let CriticalThreshold::Timeout(threshold) = critical_threshold else {
        return None;
    };
    let last_submitted_block = last_submitted_block?;
    if block_number >= last_submitted_block && block_number - last_submitted_block >= threshold {
        Some((last_submitted_block, threshold))
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
    use super::{critical_timeout_reached, CriticalThreshold};

    #[test]
    fn critical_timeout_is_reached_at_threshold() {
        assert_eq!(
            critical_timeout_reached(CriticalThreshold::Timeout(5), Some(10), 15),
            Some((10, 5))
        );
    }

    #[test]
    fn critical_timeout_is_not_reached_before_threshold() {
        assert_eq!(
            critical_timeout_reached(CriticalThreshold::Timeout(5), Some(10), 14),
            None
        );
    }

    #[test]
    fn critical_timeout_requires_last_submitted_block() {
        assert_eq!(
            critical_timeout_reached(CriticalThreshold::Timeout(5), None, 15),
            None
        );
    }

    #[test]
    fn authority_set_change_is_not_a_timeout_threshold() {
        assert_eq!(
            critical_timeout_reached(CriticalThreshold::AuthoritySetChange, Some(10), 15),
            None
        );
    }
}
