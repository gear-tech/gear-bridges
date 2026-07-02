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
    first_pending_timestamp: Option<Instant>,
    queued_root_timestamps: VecDeque<Instant>,
    merkle_root_batch: Vec<PendingMerkleRoot>,

    options: MerkleRootRelayerOptions,

    save_interval: Interval,
    main_interval: Interval,

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

        MerkleRootRelayer {
            api_provider,

            roots: HashMap::new(),
            storage,

            waiting_for_authority_set_sync: BTreeMap::new(),

            last_submitted_block: None,
            first_pending_timestamp: None,
            queued_root_timestamps: VecDeque::with_capacity(8),
            merkle_root_batch: Vec::with_capacity(8),

            options,
            save_interval,
            main_interval,

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

        let max_block_number = eth_api.max_block_number().await?;
        let max_block_distance = eth_api.max_block_distance().await?;

        if self
            .storage
            .proofs
            .get_latest_authority_set_id()
            .await
            .is_none()
        {
            log::info!(
                "Merkle root relayer {relayer_id}: proof storage is empty, syncing authority sets from genesis"
            );
            authority_set_sync.initialize();
        }

        let gear_api = self.api_provider.client();

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

                    let block = gear_api.get_block_at(block_hash).await?;
                    let block = GearBlock::from_subxt_block(&gear_api, block).await?;

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
                                return Ok(());
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

                            self.waiting_for_authority_set_sync
                                .entry(*id)
                                .or_insert_with(|| {
                                    if force_sync {
                                        authority_set_sync.send(block.clone());
                                    }
                                    Vec::new()
                                })
                                .push(block);
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
                    let signed_by_authority_set_id =
                        gear_api.signed_by_authority_set_id(block_hash).await?;
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
                        return Ok(());
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
                        return Ok(());
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

        let max_block_number_in_storage = self
            .roots
            .values()
            .filter(|r| r.proof.is_some() && matches!(r.status, MerkleRootStatus::Finalized))
            .map(|r| r.block_number)
            .max();
        let last_block_hash = self.api_provider.client().latest_finalized_block().await?;
        let last_block = self
            .api_provider
            .client()
            .block_hash_to_number(last_block_hash)
            .await?;

        log::info!("Merkle root relayer {relayer_id}: latest finalized block is #{last_block}, max block number in Ethereum MessageQueue contract is #{max_block_number} (MAX_BLOCK_DISTANCE={max_block_distance})");
        if let Some(max_stored) = max_block_number_in_storage {
            log::info!(
                "Merkle root relayer {relayer_id}: max finalized merkle root in storage is at block #{max_stored}"
            );
        } else {
            log::info!("Merkle root relayer {relayer_id}: no finalized merkle roots in storage");
        }
        // set last submitted block to the max_block_number in storage OR to max_block_number from MQ contract
        // in order to trigger catch-up logic in `run_inner` properly.
        self.last_submitted_block = Some(max_block_number_in_storage.unwrap_or(max_block_number));

        match &self.options.startup_sync_strategy {
            StartupSyncStrategy::SkipCatchUp => {
                log::info!(
                    "Merkle root relayer {relayer_id}: startup sync strategy: skip catch-up"
                );
            }

            StartupSyncStrategy::Blocks(blocks) => {
                let mut blocks = blocks.clone();
                blocks.sort_unstable();
                blocks.dedup();
                log::info!(
                    "Merkle root relayer {relayer_id}: startup sync strategy: blocks ({} block(s))",
                    blocks.len()
                );
                for block_number in blocks {
                    if block_number > last_block {
                        log::warn!(
                            "Merkle root relayer {relayer_id}: skipping startup block #{block_number} because it is higher than latest finalized block #{last_block}"
                        );
                        continue;
                    }
                    let block_hash = if block_number == last_block {
                        last_block_hash
                    } else {
                        gear_api.block_number_to_hash(block_number).await?
                    };
                    let block = gear_api.get_block_at(block_hash).await?;
                    let block = GearBlock::from_subxt_block(&gear_api, block).await?;
                    self.try_proof_merkle_root(
                        &mut prover,
                        &mut authority_set_sync,
                        block,
                        Batch::No,
                        Priority::No,
                        ForceGeneration::Yes,
                    )
                    .await?;
                }
            }

            StartupSyncStrategy::CriticalThreshold => {
                if let CriticalThreshold::Timeout(timeout) = self.options.critical_threshold {
                    if last_block >= max_block_number + timeout {
                        if let Some(max_stored) = max_block_number_in_storage {
                            // If we have some finalized merkle roots in storage, we can start from
                            // the next block that aligns with our step size to catch up.
                            let step = timeout;
                            let aligned_start = ((max_stored / step) + 1) * step;

                            // Ensure we don't start after the last block
                            let start_block = if aligned_start > last_block {
                                log::info!("Merkle root relayer {relayer_id}: aligned start block #{aligned_start} is after last block #{last_block}, processing last block only");
                                last_block
                            } else {
                                log::info!(
                                "Merkle root relayer {relayer_id}: resuming merkle root processing from block #{aligned_start} to catch up"
                            );
                                aligned_start
                            };

                            let mut block_number = start_block;
                            log::info!(
                                "Merkle root relayer {relayer_id}: processing every {step}th block to catch up"
                            );

                            // If we're starting at last_block, process it and finish
                            if start_block == last_block {
                                log::info!(
                                    "Merkle root relayer {relayer_id}: processing last block #{last_block}"
                                );
                                let block = gear_api.get_block_at(last_block_hash).await?;
                                let block = GearBlock::from_subxt_block(&gear_api, block).await?;
                                self.try_proof_merkle_root(
                                    &mut prover,
                                    &mut authority_set_sync,
                                    block,
                                    Batch::No,
                                    Priority::No,
                                    ForceGeneration::Yes,
                                )
                                .await?;
                            } else {
                                loop {
                                    log::info!(
                                        "Merkle root relayer {relayer_id}: processing block #{block_number}"
                                    );
                                    let block_hash =
                                        gear_api.block_number_to_hash(block_number).await?;
                                    let block = gear_api.get_block_at(block_hash).await?;
                                    let block =
                                        GearBlock::from_subxt_block(&gear_api, block).await?;

                                    self.try_proof_merkle_root(
                                        &mut prover,
                                        &mut authority_set_sync,
                                        block,
                                        Batch::No,
                                        Priority::No,
                                        ForceGeneration::Yes,
                                    )
                                    .await?;

                                    block_number += step;
                                    if block_number >= last_block {
                                        log::info!("Merkle root relayer {relayer_id}: reached the latest finalized block, generating merkle-root for it: #{last_block}");
                                        let block = gear_api.get_block_at(last_block_hash).await?;
                                        let block =
                                            GearBlock::from_subxt_block(&gear_api, block).await?;
                                        self.try_proof_merkle_root(
                                            &mut prover,
                                            &mut authority_set_sync,
                                            block,
                                            Batch::No,
                                            Priority::No,
                                            ForceGeneration::Yes,
                                        )
                                        .await?;
                                        break;
                                    }
                                }
                            }
                        } else if max_block_number != 0 {
                            let mut target_block = max_block_number.saturating_sub(300);
                            let step = timeout;
                            log::info!(
                                "Merkle root relayer {relayer_id}: no finalized merkle roots in storage, starting from #{target_block}"
                            );
                            loop {
                                // If there are no finalized merkle roots in storage, we need to start from
                                // max_block_number of MessageQueue contract minus some safety margin.
                                let block_hash =
                                    gear_api.block_number_to_hash(target_block).await?;
                                let block = gear_api.get_block_at(block_hash).await?;
                                let block = GearBlock::from_subxt_block(&gear_api, block).await?;

                                self.try_proof_merkle_root(
                                    &mut prover,
                                    &mut authority_set_sync,
                                    block,
                                    Batch::No,
                                    Priority::No,
                                    ForceGeneration::Yes,
                                )
                                .await?;
                                target_block += step;
                                if target_block >= last_block {
                                    log::info!("Merkle root relayer {relayer_id}: reached the latest finalized block, generating merkle-root for it: #{last_block}");
                                    let block = gear_api.get_block_at(last_block_hash).await?;
                                    let block =
                                        GearBlock::from_subxt_block(&gear_api, block).await?;
                                    self.try_proof_merkle_root(
                                        &mut prover,
                                        &mut authority_set_sync,
                                        block,
                                        Batch::No,
                                        Priority::No,
                                        ForceGeneration::Yes,
                                    )
                                    .await?;
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    let client = self.api_provider.client();
                    let max_block_hash = client.block_number_to_hash(max_block_number).await?;
                    let authority_set_eth = client.authority_set_id(max_block_hash).await?;
                    let latest_authority_set_id = client.authority_set_id(last_block_hash).await?;
                    if authority_set_eth < latest_authority_set_id {
                        log::info!("Merkle root relayer {relayer_id}: syncing authority sets from id #{authority_set_eth} to #{latest_authority_set_id}");
                        for id in authority_set_eth..=latest_authority_set_id {
                            let block_hash = client.search_for_authority_set_block(id).await?;
                            let block = client.get_block_at(block_hash).await?;
                            let block = GearBlock::from_subxt_block(&client, block).await?;
                            self.try_proof_merkle_root(
                                &mut prover,
                                &mut authority_set_sync,
                                block,
                                Batch::No,
                                Priority::No,
                                ForceGeneration::Yes,
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        if let Err(err) = self
            .run_inner(
                &mut submitter,
                &mut prover,
                &mut blocks_rx,
                &mut authority_set_sync,
                &mut http,
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

    async fn run_inner(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,

        http: &mut UnboundedReceiver<MerkleRootsRequest>,
    ) -> anyhow::Result<()> {
        loop {
            let result = self
                .process(submitter, prover, blocks_rx, authority_set_sync, http)
                .await;

            if let Err(err) = self.storage.save(&self.roots).await {
                log::error!(
                    "Merkle root relayer {}: failed to save block state: {err:?}",
                    self.options.relayer_id
                );
            }

            match result {
                Ok(true) => continue,
                Ok(false) => return Ok(()),
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
    ) -> anyhow::Result<bool> {
        let client = self.api_provider.client();
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
                                "Merkle root relayer {}: prover connection closed, exiting",
                                self.options.relayer_id
                            );
                            return Ok(false);
                        }
                    }
                }
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

                                let api = self.api_provider.client();
                                let block_hash = api.block_number_to_hash(block_number).await?;
                                let block = api.get_block_at(block_hash).await?;
                                let block = GearBlock::from_subxt_block(&client, block).await?;

                                match self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::No, Priority::Yes, ForceGeneration::Yes).await {
                                    Ok(Some((_, merkle_root))) => {
                                        if let Some(r) = self.roots.get_mut(&(block_number, merkle_root)) { r.http_requests.push(response) } else {
                                            response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).ok();
                                        }
                                    }

                                    Ok(None) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("Merkle root relayer {}: HTTP response send failed", self.options.relayer_id);
                                            return Ok(false);
                                        };
                                    }
                                    Err(err) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("Merkle root relayer {}: HTTP response send failed", self.options.relayer_id);
                                            return Ok(false);
                                        };
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
                        let mut force = ForceGeneration::No;
                        let mut batch = Batch::Yes;
                        let number = block.number();
                        if let Some(last_submitted_block) = self.last_submitted_block
                        {
                            if let CriticalThreshold::Timeout(threshold) = self.options.critical_threshold
                            {
                                if last_submitted_block + threshold <= number {
                                    log::warn!("Merkle root relayer {}: last submitted block {last_submitted_block} is older than current block number {number} by more than {threshold}, forcing proof generation", self.options.relayer_id);
                                    force = ForceGeneration::Yes;
                                    batch = Batch::No;
                                }
                            }
                        }


                        if let Some(bridging_payment_address) = self.options.bridging_payment_address {
                            for (pblock, _) in storage::priority_bridging_paid(&block, bridging_payment_address) {
                                let pblock = self.api_provider.client().get_block_at(pblock).await?;
                                let pblock = GearBlock::from_subxt_block(&client, pblock).await?;
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
                                "Merkle root relayer {}: proof submitter connection closed, exiting",
                                self.options.relayer_id
                            );
                            return Ok(false);
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
                                "Merkle root relayer {}: proof submitter connection closed, exiting",
                                self.options.relayer_id
                            );
                            return Ok(false);
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
                            let block_hash = client.search_for_authority_set_block(id).await?;
                            let block = client.get_block_at(block_hash).await?;
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
                        "Merkle root relayer {}: proof submitter connection closed, exiting",
                        self.options.relayer_id
                    );
                    return Ok(false);
                };

                self.finalize_merkle_root(response).await?;
            }
        }
        Ok(true)
    }

    async fn finalize_merkle_root(&mut self, response: submitter::Response) -> anyhow::Result<()> {
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
        let api = self.api_provider.client();

        let (queue_id, merkle_root) = if force_generation == ForceGeneration::Yes {
            api.fetch_queue_merkle_root(block.hash()).await?
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
                None => block.inclusion_proof(&api).await?,
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

                let force_sync = self
                    .storage
                    .proofs
                    .get_latest_authority_set_id()
                    .await
                    .filter(|latest| signed_by_authority_set_id > *latest)
                    .is_some();

                self.waiting_for_authority_set_sync
                    .entry(signed_by_authority_set_id)
                    .or_insert_with(|| {
                        if force_sync {
                            self.last_submitted_block = match self.last_submitted_block {
                                Some(n) if block.number() > n => Some(block.number()),
                                _ => Some(block.number()),
                            };
                            authority_set_sync.send(block.clone());
                        }
                        Vec::new()
                    })
                    .push(block);
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupSyncStrategy {
    CriticalThreshold,
    SkipCatchUp,
    Blocks(Vec<u32>),
}
