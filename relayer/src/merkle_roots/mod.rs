use crate::{
    common::{BASE_RETRY_DELAY, MAX_RETRIES},
    merkle_roots::{
        authority_set_sync::AuthoritySetSyncIo, eras::SealedNotFinalizedEra,
        prover::FinalityProverIo,
    },
    message_relayer::{
        common::{gear::block_listener::BlockListener, GearBlock},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::ProofStorageError,
    prover_interface::FinalProof,
};
use ::prover::proving::GenesisConfig;
use anyhow::Context;
use ethereum_client::EthApi;
use primitive_types::H256;
use relayer_rpc::merkle_roots::{MerkleRootsRequest, MerkleRootsResponse};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
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
    last_sealed: Option<u64>,
    genesis_config: GenesisConfig,
    eth_api: EthApi,
    rpc: UnboundedReceiver<MerkleRootsRequest>,
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
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        rpc: UnboundedReceiver<MerkleRootsRequest>,
        storage: Arc<MerkleRootStorage>,
        genesis_config: GenesisConfig,
        last_sealed: Option<u64>,
        confirmations: u64,
    ) -> Self {
        let block_listener = BlockListener::new(api_provider.clone(), storage.clone());

        let merkle_roots = MerkleRootRelayer::new(api_provider.clone(), storage.clone()).await;

        let authority_set_sync = authority_set_sync::AuthoritySetSync::new(
            api_provider.clone(),
            storage.proofs.clone(),
            genesis_config,
        )
        .await;

        let prover = prover::FinalityProver::new(api_provider.clone(), genesis_config);

        let submitter =
            submitter::MerkleRootSubmitter::new(eth_api.clone(), storage, confirmations);

        Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,
            last_sealed,
            genesis_config,
            eth_api,
            rpc,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,
            genesis_config,
            last_sealed,
            eth_api,
            rpc,
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
                rpc,
                last_sealed,
                genesis_config,
                eth_api,
            )
            .await
    }
}

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(5);

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,

    storage: Arc<MerkleRootStorage>,

    roots: HashMap<H256, MerkleRoot>,

    /// Set of blocks that are waiting for authority set sync.
    waiting_for_authority_set_sync: BTreeMap<u64, Vec<GearBlock>>,

    save_interval: Interval,
}

impl MerkleRootRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        storage: Arc<MerkleRootStorage>,
    ) -> MerkleRootRelayer {
        let mut save_interval = tokio::time::interval(Duration::from_secs(60));
        save_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        MerkleRootRelayer {
            api_provider,

            roots: HashMap::new(),
            storage,

            waiting_for_authority_set_sync: BTreeMap::new(),

            save_interval,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        mut self,
        mut blocks_rx: Receiver<GearBlock>,
        mut submitter: SubmitterIo,
        mut prover: FinalityProverIo,
        mut authority_set_sync: AuthoritySetSyncIo,
        mut rpc: UnboundedReceiver<MerkleRootsRequest>,
        last_sealed: Option<u64>,
        genesis_config: GenesisConfig,
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
        let mut last_sealed = match last_sealed {
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

        let gear_api = self.api_provider.client();

        for (hash, merkle_root) in roots.drain() {
            match &merkle_root.status {
                // most likely will need to wait for era sealing rather than authority set sync
                MerkleRootStatus::WaitForAuthoritySetSync(id, _) => {
                    log::info!(
                        "Merkle root {} for block #{} is waiting for authority set sync with id {}",
                        hash,
                        merkle_root.block_number,
                        id
                    );

                    let block = gear_api.get_block_at(merkle_root.block_hash).await?;
                    let block = GearBlock::from_subxt_block(block).await?;

                    let authority_set_proof = self
                        .storage
                        .proofs
                        .get_proof_for_authority_set_id(*id)
                        .await;
                    if let Ok(authority_set_proof) = authority_set_proof {
                        self.roots.insert(
                            hash,
                            MerkleRoot {
                                block_number: merkle_root.block_number,
                                block_hash: merkle_root.block_hash,
                                status: MerkleRootStatus::GenerateProof,
                                proof: None,
                                rpc_requests: Vec::new(),
                            },
                        );
                        if !prover.prove(
                            merkle_root.block_number,
                            merkle_root.block_hash,
                            hash,
                            authority_set_proof,
                            false,
                        ) {
                            log::error!("Prover connection closed, exiting...");
                            return Ok(());
                        }
                    } else {
                        log::warn!(
                            "Authority set proof for #{id} not found, waiting for authority set sync",
                        );
                        self.roots.insert(
                            hash,
                            MerkleRoot {
                                block_number: merkle_root.block_number,
                                block_hash: merkle_root.block_hash,
                                proof: None,
                                status: MerkleRootStatus::WaitForAuthoritySetSync(
                                    *id,
                                    block.number(),
                                ),
                                rpc_requests: Vec::new(),
                            },
                        );

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
                            .filter(|latest| *id > *latest)
                            .is_some()
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

                MerkleRootStatus::GenerateProof => {
                    self.roots.insert(
                        hash,
                        MerkleRoot {
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            status: MerkleRootStatus::GenerateProof,
                            proof: None,
                            rpc_requests: Vec::new(),
                        },
                    );

                    log::info!(
                        "Merkle root {} for block #{} is waiting for proof generation",
                        hash,
                        merkle_root.block_number
                    );

                    // if merkle root was saved in `generate proof` phase, it means
                    // that proof for authority set id is already generated and thus should be available in storage.
                    // If it is not found that is a hard error and storage should be fixed.
                    let signed_by_authority_set_id = gear_api
                        .signed_by_authority_set_id(merkle_root.block_hash)
                        .await?;
                    let inner_proof = self
                        .storage
                        .proofs
                        .get_proof_for_authority_set_id(signed_by_authority_set_id)
                        .await
                        .with_context(|| format!("Proof for authority set #{signed_by_authority_set_id} not found, please clean-up your storage and restart relayer"))?;

                    if !prover.prove(
                        merkle_root.block_number,
                        merkle_root.block_hash,
                        hash,
                        inner_proof,
                        false,
                    ) {
                        log::error!("Prover connection closed, exiting...");
                        return Ok(());
                    }
                }

                MerkleRootStatus::SubmitProof => {
                    log::info!(
                        "Merkle root {} for block #{} is waiting for proof submission",
                        hash,
                        merkle_root.block_number
                    );

                    let proof = merkle_root.proof.ok_or_else(|| {
                        anyhow::anyhow!(
                        "Proof should be available when status is SubmitProof; Check your storage"
                    )
                    })?;

                    self.roots.insert(
                        hash,
                        MerkleRoot {
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            status: MerkleRootStatus::SubmitProof,
                            proof: Some(proof.clone()),
                            rpc_requests: Vec::new(),
                        },
                    );

                    if !submitter.submit_merkle_root(merkle_root.block_number, hash, proof) {
                        log::error!("Proof submitter connection closed, exiting");
                        return Ok(());
                    }
                }

                MerkleRootStatus::Failed(ref err) => {
                    self.roots.insert(
                        hash,
                        MerkleRoot {
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            status: MerkleRootStatus::Failed(err.clone()),
                            rpc_requests: Vec::new(),
                            proof: merkle_root.proof,
                        },
                    );

                    log::error!(
                        "Merkle root {} for block #{} failed: {}",
                        hash,
                        merkle_root.block_number,
                        err
                    );
                }

                MerkleRootStatus::Finalized => {
                    self.roots.insert(
                        hash,
                        MerkleRoot {
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            status: MerkleRootStatus::Finalized,
                            rpc_requests: Vec::new(),
                            proof: merkle_root.proof,
                        },
                    );

                    log::info!(
                        "Merkle root {} for block #{} is finalized",
                        hash,
                        merkle_root.block_number
                    );
                }
            }
        }

        let mut sealed_eras = eras::Eras::new(
            Some(last_sealed),
            self.api_provider.clone(),
            eth_api,
            genesis_config,
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
                    &mut rpc,
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
        rpc: &mut UnboundedReceiver<MerkleRootsRequest>,
    ) -> anyhow::Result<()> {
        loop {
            let result = self
                .process(
                    submitter,
                    prover,
                    blocks_rx,
                    authority_set_sync,
                    sealed_eras,
                    rpc,
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
        rpc: &mut UnboundedReceiver<MerkleRootsRequest>,
    ) -> anyhow::Result<bool> {
        tokio::select! {
            _ = self.save_interval.tick() => {
                log::trace!("60 seconds passed, saving current state");
                if let Err(err) = self.storage.save(&self.roots).await {
                    log::error!("Failed to save block state: {err:?}");
                }
            }

            req = rpc.recv() => {
                match req {
                    Some(req) => {
                        match req {
                            MerkleRootsRequest::GetMerkleRootProof {
                                block_number,
                                response
                            } => {
                                // filter by `proof.is_some()` since some old storage entries do not contain proofs in them.
                                if let Some((hash, root)) = self.roots.iter().find(|(_, r)| r.block_number == block_number).filter(|(_, r)| r.proof.is_some()) {
                                    if let MerkleRootStatus::Finalized = root.status {
                                        let Ok(_) = response.send(MerkleRootsResponse::MerkleRootProof {
                                            proof: root.proof.as_ref().map(|p| p.proof.clone()).expect("proof availability is checked above"),
                                            block_number: root.block_number,
                                            block_hash: root.block_hash,
                                            merkle_root: *hash,
                                        }) else {
                                            log::error!("RPC response send failed");
                                            return Ok(false);
                                        };
                                        return Ok(true);
                                    }
                                }

                                let api = self.api_provider.client();
                                let block_hash = api.block_number_to_hash(block_number).await?;
                                let block = api.get_block_at(block_hash).await?;
                                let block = GearBlock::from_subxt_block(block).await?;

                                match self.try_proof_merkle_root(prover, authority_set_sync, block, true).await {
                                    Ok(Some(merkle_root)) => {
                                        if let Some(r) = self.roots.get_mut(&merkle_root) { r.rpc_requests.push(response) }
                                    }

                                    Ok(None) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("RPC response send failed");
                                            return Ok(false);
                                        };
                                    }

                                    Err(err) => {
                                        let Ok(_) = response.send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number }) else {
                                            log::error!("RPC response send failed");
                                            return Ok(false);
                                        };
                                        return Err(err);
                                    }
                                }
                            }
                        }
                    }

                    None => {
                        log::error!("Failed to receive RPC request");
                        return Ok(false);
                    }
                }
            }

            block = blocks_rx.recv() => {
                match block {
                    Ok(block) => {
                        self.try_proof_merkle_root(prover, authority_set_sync, block, false).await?;
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
                            "Finality proof for block #{block_number} with merkle root {merkle_root} received"
                        );

                        self.roots.entry(merkle_root)
                            .and_modify(|merkle_root| {
                                merkle_root.status = MerkleRootStatus::SubmitProof;
                                merkle_root.proof = Some(proof.clone());
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

                        for merkle_root in batch_roots {
                            log::debug!("Merkle-root {merkle_root} finalized as part of batch for block #{block_number}");
                            self.roots.entry(merkle_root)
                                .and_modify(|merkle_root| {
                                    merkle_root.status = MerkleRootStatus::Finalized;
                            });
                        }

                        self.roots.entry(merkle_root)
                            .and_modify(|merkle_root| {
                                merkle_root.status = MerkleRootStatus::SubmitProof;
                                merkle_root.proof = Some(proof.clone());
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
                            self.try_proof_merkle_root(prover, authority_set_sync, block, false).await?;
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
        if let Some(merkle_root) = self.roots.get_mut(&response.merkle_root) {
            match response.status {
                submitter::ResponseStatus::Submitted => {
                    merkle_root.status = MerkleRootStatus::Finalized;
                    log::info!(
                        "Merkle root {} for block #{} is finalized",
                        response.merkle_root,
                        response.merkle_root_block
                    );
                    let proof = merkle_root
                        .proof
                        .as_ref()
                        .map(|p| p.proof.clone())
                        .expect("proof should be available if root is finalized");
                    for req in merkle_root.rpc_requests.drain(..) {
                        let Ok(_) = req.send(MerkleRootsResponse::MerkleRootProof {
                            proof: proof.clone(),
                            block_number: merkle_root.block_number,
                            block_hash: merkle_root.block_hash,
                            merkle_root: response.merkle_root,
                        }) else {
                            log::error!("RPC response send failed");
                            return Err(anyhow::anyhow!("RPC response send failed"));
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
                    for req in merkle_root.rpc_requests.drain(..) {
                        let Ok(_) = req.send(MerkleRootsResponse::Failed {
                            message: err.clone(),
                        }) else {
                            log::error!("RPC response send failed");
                            return Err(anyhow::anyhow!("RPC response send failed"));
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
    async fn try_proof_merkle_root(
        &mut self,
        prover: &mut FinalityProverIo,
        authority_set_sync: &mut AuthoritySetSyncIo,
        block: GearBlock,
        priority: bool,
    ) -> anyhow::Result<Option<H256>> {
        let Some(merkle_root) = storage::queue_merkle_root_changed(&block) else {
            return Ok(None);
        };
        // mark root processed so that we don't process the entire block again.
        self.storage.merkle_root_processed(block.number()).await;

        if let Err(err) = self.storage.save(&self.roots).await {
            log::error!("Failed to save block storage state: {err:?}");
        }

        if self.roots.contains_key(&merkle_root)
            || self.storage.is_merkle_root_submitted(merkle_root).await
        {
            log::info!(
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

        match self
            .storage
            .proofs
            .get_proof_for_authority_set_id(signed_by_authority_set_id)
            .await
        {
            Ok(inner_proof) => {
                let number = block.number();
                self.roots.insert(
                    merkle_root,
                    MerkleRoot {
                        block_number: number,
                        block_hash: block.hash(),
                        status: MerkleRootStatus::GenerateProof,
                        rpc_requests: Vec::new(),
                        proof: None,
                    },
                );
                log::info!("Proof for authority set #{signed_by_authority_set_id} is found, generating proof for merkle-root {merkle_root} at block #{number}");
                if !prover.prove(
                    block.number(),
                    block.hash(),
                    merkle_root,
                    inner_proof,
                    priority,
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
                self.roots.insert(
                    merkle_root,
                    MerkleRoot {
                        block_number: block.number(),
                        block_hash: block.hash(),
                        status: MerkleRootStatus::WaitForAuthoritySetSync(
                            signed_by_authority_set_id,
                            block.number(),
                        ),
                        rpc_requests: Vec::new(),
                        proof: None,
                    },
                );

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
                    merkle_root,
                    MerkleRoot {
                        block_number: block.number(),
                        block_hash: block.hash(),
                        status: MerkleRootStatus::Failed(err.to_string()),
                        rpc_requests: Vec::new(),
                        proof: None,
                    },
                );
                log::error!(
                    "Failed to get proof for authority set id {signed_by_authority_set_id}: {err}"
                );
                return Err(err.into());
            }
        }

        Ok(Some(merkle_root))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MerkleRoot {
    pub block_number: u32,
    pub block_hash: H256,
    #[serde(skip)]
    pub rpc_requests: Vec<tokio::sync::oneshot::Sender<MerkleRootsResponse>>,
    #[serde(default)]
    pub proof: Option<FinalProof>,
    pub status: MerkleRootStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MerkleRootStatus {
    WaitForAuthoritySetSync(u64, u32),
    GenerateProof,
    SubmitProof,
    Finalized,
    Failed(String),
}
