use crate::{
    cli::{GearEthCoreArgs, DEFAULT_COUNT_CONFIRMATIONS, DEFAULT_COUNT_THREADS},
    common::{BASE_RETRY_DELAY, MAX_RETRIES},
    hex_utils,
    merkle_roots::{
        authority_set_sync::AuthoritySetSyncIo, eras::SealedNotFinalizedEra,
        prover::FinalityProverIo,
    },
    message_relayer::{
        common::{
            gear::block_listener::BlockListener,
            web_request::{MerkleRootsRequest, MerkleRootsResponse},
            GearBlock,
        },
        eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::ProofStorageError,
    prover_interface::FinalProof,
};
use ::prover::{
    consts::BLAKE2_DIGEST_SIZE,
    proving::{GenesisConfig, ProofWithCircuitData},
};
use anyhow::Context;
use ethereum_client::EthApi;
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
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
use utils_prometheus::MeteredService;

pub mod authority_set_sync;
pub mod eras;
pub mod prover;
pub mod storage;
pub mod submitter;

pub struct Relayer {
    merkle_roots: MerkleRootRelayer,
    authority_set_sync: authority_set_sync::AuthoritySetSync,
    prover: prover::FinalityProver,
    submitter: submitter::MerkleRootSubmitter,
    block_listener: BlockListener,

    eth_api: EthApi,
    http: UnboundedReceiver<MerkleRootsRequest>,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.authority_set_sync
            .get_sources()
            .into_iter()
            .chain(self.submitter.get_sources())
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
        let block_listener = BlockListener::new(api_provider.clone(), storage.clone());

        let merkle_roots =
            MerkleRootRelayer::new(api_provider.clone(), storage.clone(), options).await;

        let authority_set_sync = authority_set_sync::AuthoritySetSync::new(
            api_provider.clone(),
            storage.proofs.clone(),
            options.genesis_config,
            options.count_thread,
        )
        .await;

        let prover = prover::FinalityProver::new(
            api_provider.clone(),
            options.genesis_config,
            options.count_thread,
        );

        let submitter =
            submitter::MerkleRootSubmitter::new(eth_api.clone(), storage, options.confirmations);

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

        //let sealed_eras = eras.seal(merkle_roots.storage.proofs.clone());
        let authority_set_sync = authority_set_sync.run(blocks1);
        let prover = prover.run();
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

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(5);

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,

    storage: Arc<MerkleRootStorage>,

    roots: HashMap<(u32, H256), MerkleRoot>,

    /// Set of blocks that are waiting for authority set sync.
    waiting_for_authority_set_sync: BTreeMap<u64, Vec<GearBlock>>,

    last_submitted_timestamp: Option<u64>,
    first_pending_timestamp: Option<Instant>,
    queued_root_timestamps: VecDeque<Instant>,
    merkle_root_batch: Vec<PendingMerkleRoot>,

    options: MerkleRootRelayerOptions,

    save_interval: Interval,
    main_interval: Interval,
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

            last_submitted_timestamp: None,
            first_pending_timestamp: None,
            queued_root_timestamps: VecDeque::with_capacity(8),
            merkle_root_batch: Vec::with_capacity(8),

            options,
            save_interval,
            main_interval,
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
        log::info!("Starting relayer");
        let mut roots = match self.storage.load().await {
            Ok(roots) => roots,
            Err(err) => {
                log::error!("Failed to load merkle roots from storage: {err}");
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

        if self
            .storage
            .proofs
            .get_latest_authority_set_id()
            .await
            .is_none()
        {
            log::info!("Proof storage is empty, syncing authority sets from genesis");
            authority_set_sync.initialize();
        }

        let gear_api = self.api_provider.client();

        for ((block_number, hash), merkle_root) in roots.drain() {
            let block_hash = merkle_root.block_hash;
            let timestamp = merkle_root.timestamp;

            let mut reinstate = |status: MerkleRootStatus| {
                self.roots.insert(
                    (block_number, hash),
                    MerkleRoot {
                        queue_id: 0,
                        timestamp,
                        block_number,
                        block_hash,
                        status,
                        message_nonces: Vec::new(),
                        proof: merkle_root.proof.clone(),
                        http_requests: Vec::new(),
                    },
                );
            };

            match &merkle_root.status {
                // most likely will need to wait for era sealing rather than authority set sync
                MerkleRootStatus::WaitForAuthoritySetSync(id, _) => {
                    log::info!(
                        "Merkle root {hash} for block #{block_number} is waiting for authority set sync with id {id}"
                    );

                    let block = gear_api.get_block_at(block_hash).await?;
                    let block = GearBlock::from_subxt_block(block).await?;

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
                                false,
                                true,
                            ) {
                                log::error!("Prover connection closed, exiting...");
                                return Ok(());
                            }
                        }
                        Err(_) => {
                            log::warn!("Authority set proof for #{id} not found, waiting for authority set sync");

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
                        "Merkle root {hash} for block #{block_number} is waiting for proof generation"
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
                        false,
                        true,
                    ) {
                        log::error!("Prover connection closed, exiting...");
                        return Ok(());
                    }
                }

                MerkleRootStatus::SubmitProof => {
                    log::info!(
                        "Merkle root {hash} for block #{block_number} is waiting for proof submission"
                    );
                    let proof = merkle_root
                        .proof
                        .clone()
                        .expect("proof should be available if root is in SubmitProof state; check your storage");

                    reinstate(MerkleRootStatus::SubmitProof);

                    if !submitter.submit_merkle_root(block_number, hash, proof) {
                        log::error!("Proof submitter connection closed, exiting");
                        return Ok(());
                    }
                }

                MerkleRootStatus::Failed(err) => {
                    reinstate(MerkleRootStatus::Failed(err.clone()));

                    log::error!("Merkle root {hash} for block #{block_number} failed: {err}");
                }

                MerkleRootStatus::Finalized => {
                    reinstate(MerkleRootStatus::Finalized);

                    log::info!("Merkle root {hash} for block #{block_number} is finalized");
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

        log::info!("Latest finalized block is #{last_block}, max block number in Ethereum MessageQueue contract is #{max_block_number}");
        if let Some(max_stored) = max_block_number_in_storage {
            log::info!("Max finalized merkle root in storage is at block #{max_stored}");
        } else {
            log::info!("No finalized merkle roots in storage");
        }

        if last_block > max_block_number {
            if let Some(max_stored) = max_block_number_in_storage {
                // If we have some finalized merkle roots in storage, we can start from
                // max of (max_block_number_in_storage + 1) and (last finalized block - 300).
                let start_block = (max_stored + 1).max(last_block.saturating_sub(300));
                log::info!("Resuming merkle root processing from block #{start_block} to catch up");
                let mut block_number = start_block;
                // step is a number of blocks to skip ahead
                let step = (self.options.critical_threshold.as_secs() / 3) as u32;
                log::info!("Processing every {step}th block to catch up");
                loop {
                    log::info!("Processing block #{block_number}");
                    let block_hash = gear_api.block_number_to_hash(block_number).await?;
                    let block = gear_api.get_block_at(block_hash).await?;
                    let block = GearBlock::from_subxt_block(block).await?;
                    let timestamp = gear_api.fetch_timestamp(block.hash()).await?;

                    self.try_proof_merkle_root(
                        &mut prover,
                        &mut authority_set_sync,
                        block,
                        Batch::No,
                        Priority::No,
                        ForceGeneration::Yes,
                        timestamp,
                    )
                    .await?;
                    block_number += step;
                    if block_number >= last_block {
                        log::info!("Reached the latest finalized block, generating merkle-root for it: #{last_block}");
                        let block = gear_api.get_block_at(last_block_hash).await?;
                        let block = GearBlock::from_subxt_block(block).await?;
                        self.try_proof_merkle_root(
                            &mut prover,
                            &mut authority_set_sync,
                            block,
                            Batch::No,
                            Priority::No,
                            ForceGeneration::Yes,
                            timestamp,
                        )
                        .await?;
                        break;
                    }
                }
            } else if max_block_number != 0 {
                let target_block = max_block_number.saturating_sub(300);
                log::info!("No finalized merkle roots in storage, starting from #{target_block}");

                // If there are no finalized merkle roots in storage, we need to start from
                // max_block_number of MessageQueue contract minus some safety margin.
                let block_hash = gear_api.block_number_to_hash(target_block).await?;
                let block = gear_api.get_block_at(block_hash).await?;
                let block = GearBlock::from_subxt_block(block).await?;
                let timestamp = gear_api.fetch_timestamp(block.hash()).await?;

                self.try_proof_merkle_root(
                    &mut prover,
                    &mut authority_set_sync,
                    block,
                    Batch::No,
                    Priority::No,
                    ForceGeneration::Yes,
                    timestamp,
                )
                .await?;
            }
        }

        let mut sealed_eras = eras::Eras::new(
            Some(last_sealed),
            self.api_provider.clone(),
            eth_api,
            self.options.genesis_config,
            self.options.count_thread,
        )
        .await?
        .seal(self.storage.proofs.clone());

        let mut attempts = 0;

        loop {
            attempts += 1;
            let now = Instant::now();

            if let Err(err) = self
                .run_inner(
                    &mut submitter,
                    &mut prover,
                    &mut blocks_rx,
                    &mut authority_set_sync,
                    &mut sealed_eras,
                    &mut http,
                )
                .await
            {
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                log::error!(
                    "Main loop error (attempt {attempts}/{MAX_RETRIES}): {err}. Retrying in {delay:?}..."
                );
                if attempts >= MAX_RETRIES {
                    log::error!("Max attempts reached. Exiting...");
                    return Err(err.context("Max attempts reached"));
                }
                tokio::time::sleep(delay).await;

                match self.api_provider.reconnect().await {
                    Ok(()) => {
                        log::info!("Merkle root relayer reconnected successfully");
                    }

                    Err(err) => {
                        log::error!("Failed to reconnect to Gear API: {err}");
                        return Err(err.context("Failed to reconnect to Gear API"));
                    }
                }
            } else {
                log::warn!("Gear block listener connection closed, exiting");
                return Ok(());
            }

            let main_loop_duration = now.elapsed();
            if main_loop_duration < MIN_MAIN_LOOP_DURATION {
                tokio::time::sleep(MIN_MAIN_LOOP_DURATION - main_loop_duration).await;
            }
        }
    }

    async fn run_inner(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,
        sealed_eras: &mut UnboundedReceiver<SealedNotFinalizedEra>,
        http: &mut UnboundedReceiver<MerkleRootsRequest>,
    ) -> anyhow::Result<()> {
        loop {
            let result = self
                .process(
                    submitter,
                    prover,
                    blocks_rx,
                    authority_set_sync,
                    sealed_eras,
                    http,
                )
                .await;

            if let Err(err) = self.storage.save(&self.roots).await {
                log::error!("Failed to save block state: {err:?}");
            }

            match result {
                Ok(true) => continue,
                Ok(false) => return Ok(()),
                Err(err) => {
                    log::error!("Error processing blocks: {err}");
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
        sealed_eras: &mut UnboundedReceiver<SealedNotFinalizedEra>,
        http: &mut UnboundedReceiver<MerkleRootsRequest>,
    ) -> anyhow::Result<bool> {
        tokio::select! {
            _ = self.save_interval.tick() => {
                log::trace!("60 seconds passed, saving current state");
                if let Err(err) = self.storage.save(&self.roots).await {
                    log::error!("Failed to save block state: {err:?}");
                }
            }

            _ = self.main_interval.tick() => {
                // prune old timestamps to not trigger spike when not necessary
                self.prune_old_timestamps();

                let is_spike = self.merkle_root_batch.iter().map(|root| root.nonces_count).sum::<usize>() >= self.options.spike_config.threshold;
                let is_timeout = self.first_pending_timestamp
                    .is_some_and(|t| t.elapsed() >= self.options.spike_config.timeout);

                if is_spike || is_timeout {
                    // consume the timestamp to not trigger timeout again immediately.
                    self.first_pending_timestamp.take();
                    let batch_size = self.merkle_root_batch.len();
                    if batch_size == 0 {
                        return Ok(true);
                    }
                    log::info!("Triggering proof generation. Batch size: {batch_size}, Reason: Spike={is_spike}, Timeout={is_timeout}");
                    // do not group blocks by authority set id, prover will do this for us.
                    for pending in self.merkle_root_batch.drain(..) {
                        if !prover.prove(
                            pending.block_number,
                            pending.block_hash,
                            pending.merkle_root,
                            pending.inner_proof,
                            pending.queue_id,
                            false,
                            /* request is part of the batch: */
                            true,
                        ) {
                            log::warn!("Prover connection closed, exiting");
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
                                            log::error!("HTTP response send failed");
                                            return Ok(false);
                                        };
                                        return Ok(true);
                                    }
                                }

                                let api = self.api_provider.client();
                                let block_hash = api.block_number_to_hash(block_number).await?;
                                let block = api.get_block_at(block_hash).await?;
                                let block = GearBlock::from_subxt_block(block).await?;
                                let timestamp = api.fetch_timestamp(block.hash()).await?;

                                match self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::No, Priority::Yes, ForceGeneration::Yes, timestamp).await {
                                    Ok(Some((_, merkle_root))) => {
                                        if let Some(r) = self.roots.get_mut(&(block_number, merkle_root)) { r.http_requests.push(response) } else {
                                            response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }).ok();
                                        }
                                    }

                                    Ok(None) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("HTTP response send failed");
                                            return Ok(false);
                                        };
                                    }
                                    Err(err) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("HTTP response send failed");
                                            return Ok(false);
                                        };
                                        return Err(err);
                                    }
                                }
                            }
                        }
                    }


                    None => {
                        log::error!("Failed to receive HTTP request");
                        return Ok(false);
                    }
                }
            }

            block = blocks_rx.recv() => {
                match block {
                    Ok(block) => {
                        let mut force = ForceGeneration::No;
                        let mut batch = Batch::Yes;
                        let timestamp = self.api_provider.client().fetch_timestamp(block.hash()).await?;
                        if let Some(last_submitted_timestamp) = self.last_submitted_timestamp {
                            if last_submitted_timestamp + self.options.critical_threshold.as_secs() <= timestamp {
                                log::warn!("Last submitted timestamp {last_submitted_timestamp} is older than current block timestamp {timestamp} by more than {threshold:?}, forcing proof generation", threshold = self.options.critical_threshold.as_secs());
                                force = ForceGeneration::Yes;
                                batch = Batch::No;
                            }
                        }

                        if let Some(bridging_payment_address) = self.options.bridging_payment_address {
                            for (pblock, _) in storage::priority_bridging_paid(&block, bridging_payment_address) {
                                let pblock = self.api_provider.client().get_block_at(pblock).await?;
                                let pblock = GearBlock::from_subxt_block(pblock).await?;
                                log::info!("Priority bridging requested at block #{}, generating proof for merkle-root at block #{}", block.number(), pblock.number());
                                let timestamp = self.api_provider.client().fetch_timestamp(pblock.hash()).await?;
                                self.try_proof_merkle_root(prover, authority_set_sync, pblock, Batch::No, Priority::Yes, ForceGeneration::Yes, timestamp).await?;
                            }
                        }

                        self.try_proof_merkle_root(prover, authority_set_sync, block, batch, Priority::No, force, timestamp).await?;
                    }

                    Err(RecvError::Lagged(n)) => {
                        log::warn!("Merkle root relayer lagged behind {n} blocks");
                        return Ok(true);
                    }

                    Err(RecvError::Closed) => {
                        log::warn!("Block listener connection closed, exiting");
                        return Ok(false);
                    }
                }
            }

            response = prover.recv() => {
                let Some(response) = response else {
                    log::warn!("Finality prover connection closed, exiting");
                    return Ok(false);
                };

                match response {
                    prover::Response::Single {
                        block_number,
                        merkle_root,
                        proof,
                    } => {
                        log::info!(
                            "Finality proof for block #{block_number} with merkle root {merkle_root} received");

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
                                        log::error!("RPC response send failed");
                                        continue;
                                    };
                                }
                            });


                        if !submitter.submit_merkle_root(block_number, merkle_root, proof) {
                            log::warn!("Proof submitter connection closed, exiting");
                            return Ok(false);
                        }
                    }

                    prover::Response::Batched {
                        block_number,
                        merkle_root,
                        proof,
                        batch_roots
                    } => {
                        log::info!("Finality proof for block #{block_number} with merkle root {merkle_root} received (will apply to {} blocks)", batch_roots.len());

                        for (block_number, merkle_root) in batch_roots {
                            log::debug!("Merkle-root {merkle_root} finalized as part of batch for block #{block_number}");
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
                                            log::error!("RPC response send failed");
                                            continue;
                                        };
                                        log::info!("Send HTTP response for merkle root {merkle_root} at block #{block_number}");
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
                                        log::error!("RPC response send failed");
                                        continue;
                                    };
                                }
                            });

                        if !submitter.submit_merkle_root(block_number, merkle_root, proof) {
                            log::warn!("Proof submitter connection closed, exiting");
                            return Ok(false);
                        }
                    }
                }
            }


            Some(sealed_era) = sealed_eras.recv(), if !sealed_eras.is_closed() => {
                log::info!(
                    "Sealed era #{} at block #{} with merkle root {} received",
                    sealed_era.era,
                    sealed_era.merkle_root_block,
                    H256::from(sealed_era.proof.merkle_root)
                );

                if !submitter.submit_era_root(sealed_era.era, sealed_era.merkle_root_block, sealed_era.proof) {
                    log::warn!("Proof submitter connection closed, exiting");
                    return Ok(false);
                }
            }

            response = authority_set_sync.recv() => {
                let Some(response) = response else {
                    log::warn!("Authority set sync connection closed, exiting");
                    return Ok(false);
                };

                match response {
                    authority_set_sync::Response::AuthoritySetSynced(id, block) => {
                        self.storage.authority_set_processed(block).await;

                        let Some(mut to_submit) = self.waiting_for_authority_set_sync.remove(&id) else {
                            log::warn!("No blocks to sync for authority set #{id}");
                            return Ok(true)
                        };

                        log::info!("Authority set #{id} is synced, submitting {} blocks", to_submit.len());
                        while let Some(block) = to_submit.pop() {
                            let timestamp = self.api_provider.client().fetch_timestamp(block.hash()).await?;
                            self.try_proof_merkle_root(prover, authority_set_sync, block, Batch::No, Priority::No, ForceGeneration::No, timestamp).await?;
                        }
                    }
                }
            }

            response = submitter.recv() => {
                let Some(response) = response else {
                    log::warn!("Proof submitter connection closed, exiting");
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
                "Era #{} merkle root {} for block #{} is finalized with status: {:?}",
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
                    self.last_submitted_timestamp = match self.last_submitted_timestamp {
                        Some(ts) if merkle_root.timestamp > ts => Some(merkle_root.timestamp),
                        _ => Some(merkle_root.timestamp),
                    };
                    merkle_root.status = MerkleRootStatus::Finalized;
                    log::info!(
                        "Merkle root {} for block #{} is finalized",
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
                            log::error!("HTTP response send failed");
                            return Err(anyhow::anyhow!("HTTP response send failed"));
                        };
                    }
                }

                submitter::ResponseStatus::Failed(err) => {
                    merkle_root.status = MerkleRootStatus::Failed(err.to_string());
                    log::error!(
                        "Failed to finalize merkle root {} for block #{}: {}",
                        response.merkle_root,
                        response.merkle_root_block,
                        err
                    );
                    for req in merkle_root.http_requests.drain(..) {
                        let Ok(_) = req.send(MerkleRootsResponse::Failed {
                            message: err.clone(),
                        }) else {
                            log::error!("HTTP response send failed");
                            return Err(anyhow::anyhow!("HTTP response send failed"));
                        };
                    }
                }
            }
        } else {
            log::warn!(
                "Merkle root {} for block #{} not found in storage",
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
        timestamp: u64,
    ) -> anyhow::Result<Option<(u64, H256)>> {
        let (queue_id, merkle_root) = if force_generation == ForceGeneration::Yes {
            self.api_provider
                .client()
                .fetch_queue_merkle_root(block.hash())
                .await?
        } else {
            match storage::queue_merkle_root_changed(&block) {
                Some(merkle_root) => merkle_root,
                None => {
                    log::debug!(
                        "Skipping block #{} as there are no new messages",
                        block.number()
                    );
                    return Ok(None);
                }
            }
        };

        let nonces = storage::message_queued_events_of(&block).collect::<Vec<_>>();

        // mark root processed so that we don't process the entire block again.
        self.storage.merkle_root_processed(block.number()).await;

        if let Err(err) = self.storage.save(&self.roots).await {
            log::error!("Failed to save block storage state: {err:?}");
        }

        if self
            .storage
            .is_merkle_root_submitted(block.number(), merkle_root)
            .await
            && force_generation == ForceGeneration::No
        {
            log::debug!(
                "Skipping merkle root {} for block #{} as there were no new messages",
                merkle_root,
                block.number()
            );
            return Ok(None);
        }

        let signed_by_authority_set_id = self
            .api_provider
            .client()
            .signed_by_authority_set_id(block.hash())
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
                self.last_submitted_timestamp = match self.last_submitted_timestamp {
                    Some(ts) if timestamp > ts => Some(timestamp),
                    _ => Some(timestamp),
                };
                self.roots
                    .entry((block_number, merkle_root))
                    .or_insert(MerkleRoot {
                        queue_id,
                        timestamp,
                        block_number,
                        block_hash,
                        status: MerkleRootStatus::GenerateProof,
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
                    });
                if matches!(batch, Batch::Yes) {
                    let now = Instant::now();

                    if self.merkle_root_batch.is_empty() {
                        self.first_pending_timestamp = Some(now);
                    }

                    log::info!("Merkle-root #{merkle_root} at block #{block_number} with queue #{queue_id} is enqueued for batch processing");

                    self.queued_root_timestamps.push_back(now);
                    self.merkle_root_batch.push(PendingMerkleRoot {
                        block_hash,
                        block_number,
                        merkle_root,
                        inner_proof,
                        nonces_count,
                        queue_id,
                    });
                    return Ok(Some((queue_id, merkle_root)));
                }
                log::info!("Proof for authority set #{signed_by_authority_set_id} is found, generating proof for merkle-root {merkle_root} at block #{block_number} with queue #{queue_id}");
                if !prover.prove(
                    block_number,
                    block_hash,
                    merkle_root,
                    inner_proof,
                    queue_id,
                    priority == Priority::Yes,
                    /* non batching request: should be processed separately */
                    false,
                ) {
                    log::error!("Prover connection closed, exiting...");
                    return Err(anyhow::anyhow!("Prover connection closed"));
                }
            }

            Err(ProofStorageError::NotInitialized) | Err(ProofStorageError::NotFound(_)) => {
                log::info!(
                    "Delaying proof generation for merkle root {} at block #{} until authority set #{} is synced",
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
                        timestamp,
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
                        timestamp,
                        block_number: block.number(),
                        block_hash: block.hash(),
                        status: MerkleRootStatus::Failed(err.to_string()),
                        message_nonces: nonces,
                        http_requests: Vec::new(),
                        proof: None,
                    },
                );

                log::error!(
                    "Failed to get proof for authority set id {signed_by_authority_set_id}: {err}"
                );
                return Err(err.into());
            }
        }

        Ok(Some((queue_id, merkle_root)))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MerkleRoot {
    pub block_number: u32,
    pub block_hash: H256,
    pub queue_id: u64,
    pub timestamp: u64,
    pub message_nonces: Vec<U256>,
    #[serde(skip)]
    pub http_requests: Vec<tokio::sync::oneshot::Sender<MerkleRootsResponse>>,
    #[serde(default)]
    pub proof: Option<FinalProof>,
    pub status: MerkleRootStatus,
}

impl Clone for MerkleRoot {
    fn clone(&self) -> Self {
        Self {
            block_number: self.block_number,
            block_hash: self.block_hash,
            queue_id: self.queue_id,
            timestamp: self.timestamp,
            message_nonces: self.message_nonces.clone(),
            http_requests: Vec::new(),
            proof: self.proof.clone(),
            status: self.status.clone(),
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

#[derive(Clone, Copy)]
pub struct MerkleRootRelayerOptions {
    pub spike_config: SpikeConfig,
    pub check_interval: Duration,
    pub save_interval: Duration,
    pub genesis_config: GenesisConfig,
    pub last_sealed: Option<u64>,
    pub confirmations: u64,
    pub count_thread: Option<usize>,
    pub bridging_payment_address: Option<H256>,
    pub critical_threshold: Duration,
}

impl MerkleRootRelayerOptions {
    pub fn from_cli(config: &GearEthCoreArgs) -> anyhow::Result<Self> {
        Ok(Self {
            critical_threshold: config.critical_threshold,
            spike_config: SpikeConfig {
                timeout: config.spike_timeout,
                window: config.spike_window,
                threshold: config.spike_threshold,
            },
            check_interval: config.check_interval,
            save_interval: config.save_interval,
            genesis_config: GenesisConfig {
                authority_set_hash: hex::decode(&config.genesis_config_args.authority_set_hash)
                    .context(
                        "Incorrect format for authority set hash: hex encoded hash is expected",
                    )?
                    .try_into()
                    .map_err(|got: Vec<u8>| {
                        anyhow::anyhow!("Incorrect format for authority set hash: wrong length. Expected {}, got {}", BLAKE2_DIGEST_SIZE, got.len())
                    })?,
                authority_set_id: config.genesis_config_args.authority_set_id,
            },
            last_sealed: config.start_authority_set_id,
            confirmations: config.confirmations_merkle_root.unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
            count_thread: match config.thread_count {
                None => Some(DEFAULT_COUNT_THREADS),
                Some(thread_count) => thread_count.into(),
            },
            bridging_payment_address: config
                .bridging_payment_address
                .as_ref()
                .map(|x| hex_utils::decode_h256(x))
                .transpose()
                .context("Failed to parse bridging payment address")?,
        })
    }
}

#[derive(Copy, Clone)]
pub struct SpikeConfig {
    /// Timeout after which we start relaying merkle-root
    pub timeout: Duration,
    /// Spike window, used to cutoff old merkle-roots
    pub window: Duration,
    /// Spike threshold: after threshold is reached we enter "spike"
    /// mode where proofs are generated immediately.
    pub threshold: usize,
}

impl Default for SpikeConfig {
    fn default() -> Self {
        Self {
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
