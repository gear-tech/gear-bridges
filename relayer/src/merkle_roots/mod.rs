use crate::{
    common::{BASE_RETRY_DELAY, MAX_RETRIES},
    merkle_roots::{authority_set_sync::AuthoritySetSyncIo, prover::FinalityProverIo},
    message_relayer::{
        common::gear::block_listener::{BlockListener, GearBlock},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::{ProofStorage, ProofStorageError},
};
use ::prover::proving::GenesisConfig;
use ethereum_client::EthApi;
use gclient::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::H256;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use submitter::SubmitterIo;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use utils_prometheus::MeteredService;

pub mod authority_set_sync;
pub mod prover;
pub mod submitter;

pub struct Relayer {
    merkle_roots: MerkleRootRelayer,
    authority_set_sync: authority_set_sync::AuthoritySetSync,
    prover: prover::FinalityProver,
    submitter: submitter::MerkleRootSubmitter,
    block_listener: BlockListener,
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
        proof_storage: Arc<dyn ProofStorage>,
        genesis_config: GenesisConfig,
        last_sealed: Option<u64>,
    ) -> Self {
        let block_listener = BlockListener::new(api_provider.clone());

        let merkle_roots =
            MerkleRootRelayer::new(api_provider.clone(), proof_storage.clone()).await;

        let authority_set_sync = authority_set_sync::AuthoritySetSync::new(
            api_provider.clone(),
            eth_api.clone(),
            proof_storage.clone(),
            last_sealed,
            genesis_config,
        )
        .await;

        let prover = prover::FinalityProver::new(api_provider.clone(), genesis_config);

        let submitter = submitter::MerkleRootSubmitter::new(eth_api.clone());

        Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let Self {
            merkle_roots,
            authority_set_sync,
            prover,
            submitter,
            block_listener,
        } = self;

        let [blocks0, blocks1] = block_listener.run().await;

        let authority_set_sync = authority_set_sync.run(blocks1);
        let prover = prover.run();
        let submitter = submitter.run();

        merkle_roots
            .run(blocks0, submitter, prover, authority_set_sync)
            .await
    }
}

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(5);

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,

    proof_storage: Arc<dyn ProofStorage>,

    /// Set of blocks that are waiting for authority set sync.
    waiting_for_authority_set_sync: BTreeMap<u64, Vec<GearBlock>>,
    last_submitted_merkle_root: Option<H256>,
}

impl MerkleRootRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        proof_storage: Arc<dyn ProofStorage>,
    ) -> MerkleRootRelayer {
        MerkleRootRelayer {
            api_provider,

            proof_storage,
            waiting_for_authority_set_sync: BTreeMap::new(),
            last_submitted_merkle_root: None,
        }
    }

    pub async fn run(
        mut self,
        mut blocks_rx: Receiver<GearBlock>,
        mut submitter: SubmitterIo,
        mut prover: FinalityProverIo,
        mut authority_set_sync: AuthoritySetSyncIo,
    ) -> anyhow::Result<()> {
        log::info!("Starting relayer");

        let mut attempts = 0;

        loop {
            attempts += 1;
            let now = Instant::now();

            if let Err(err) = self
                .process(
                    &mut submitter,
                    &mut prover,
                    &mut blocks_rx,
                    &mut authority_set_sync,
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

    fn queue_merkle_root_changed(block: &GearBlock) -> Option<H256> {
        block.events().iter().find_map(|event| match event {
            gclient::Event::GearEthBridge(GearEthBridgeEvent::QueueMerkleRootChanged(root)) => {
                Some(*root)
            }
            _ => None,
        })
    }

    async fn process(
        &mut self,
        submitter: &mut SubmitterIo,
        prover: &mut FinalityProverIo,
        blocks_rx: &mut Receiver<GearBlock>,
        authority_set_sync: &mut AuthoritySetSyncIo,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                block = blocks_rx.recv() => {
                    match block {
                        Ok(block) => {
                            if !self.try_proof_merkle_root(prover, block).await? {
                                return Ok(());
                            }
                        }

                        Err(RecvError::Lagged(n)) => {
                            log::warn!("Merkle root relayer lagged behind {n} blocks");
                            continue;
                        }

                        Err(RecvError::Closed) => {
                            log::warn!("Block listener connection closed, exiting");
                            return Ok(());
                        }
                    }
                }

                response = prover.recv() => {
                    let Some(response) = response else {
                       log::warn!("Finality prover connection closed, exiting");
                       return Ok(());
                    };

                    if !submitter.submit_merkle_root(response.block_number, response.proof) {
                        log::warn!("Proof submitter connection closed, exiting");
                        return Ok(());
                    }
                }

                response = authority_set_sync.recv() => {
                    let Some(response) = response else {
                        log::warn!("Authority set sync connection closed, exiting");
                        return Ok(());
                    };

                    log::info!("Submitting merkle-root proof for era #{} at block #{}", response.era, response.merkle_root_block);
                    if !submitter.submit_era_root(response.era, response.merkle_root_block, response.proof) {
                        log::warn!("Proof submitter connection closed, exiting");
                        return Ok(());
                    }
                }

                response = submitter.recv() => {
                    let Some(response) = response else {
                        log::warn!("Proof submitter connection closed, exiting");
                        return Ok(());
                    };

                    let Some(era) = response.era else {
                        continue;
                    };

                    let Some(mut to_submit) = self.waiting_for_authority_set_sync.remove(&era) else {
                        log::warn!("No blocks to sync for authority set id {era}");
                        continue;
                    };

                    log::info!("Era #{era} is synced, submitting {} blocks", to_submit.len());
                    while let Some(block) = to_submit.pop() {
                        if !self.try_proof_merkle_root(prover, block).await? {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Attempt to create proof for merkle root of `block`. If authority set id
    /// that signed `block`, proof generation will be delayed until authority set is synced.
    async fn try_proof_merkle_root(
        &mut self,
        prover: &mut FinalityProverIo,
        block: GearBlock,
    ) -> anyhow::Result<bool> {
        let Some(merkle_root) = Self::queue_merkle_root_changed(&block) else {
            return Ok(true);
        };

        if self.last_submitted_merkle_root == Some(merkle_root) {
            log::info!(
                "Skipping merkle root {} for block #{} as there were no new messages",
                merkle_root,
                block.number()
            );
            return Ok(true);
        }

        let signed_by_authority_set_id = self
            .api_provider
            .client()
            .signed_by_authority_set_id(block.hash())
            .await?;

        match self
            .proof_storage
            .get_proof_for_authority_set_id(signed_by_authority_set_id)
            .await
        {
            Ok(inner_proof) => {
                if !prover.prove(block.number(), block.hash(), merkle_root, inner_proof) {
                    return Ok(false);
                }
            }

            Err(ProofStorageError::NotFound(_)) => {
                log::info!(
                    "Delaying proof generation for merkle root {} at block #{} until authority set #{} is synced",
                    merkle_root,
                    block.number(),
                    signed_by_authority_set_id,
                );
                self.waiting_for_authority_set_sync
                    .entry(signed_by_authority_set_id)
                    .or_default()
                    .push(block);
            }

            Err(err) => {
                log::error!(
                    "Failed to get proof for authority set id {signed_by_authority_set_id}: {err}"
                );
                return Err(err.into());
            }
        }

        Ok(true)
    }
}
