use authority_set_sync::SealedNotFinalizedEra;
use prometheus::{Gauge, IntGauge};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use submitter::SubmitterIo;

use crate::{
    common::{self, sync_authority_set_id, SyncStepCount, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::gear::block_listener::GearBlock, eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};
use ::prover::proving::GenesisConfig;
use ethereum_client::EthApi;
use gclient::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::H256;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(5);

impl_metered_service! {
    struct Metrics {
        latest_proven_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_proven_era",
            "Latest proven era number",
        ),
        latest_observed_gear_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
        fee_payer_balance: Gauge = Gauge::new(
            "merkle_root_relayer_fee_payer_balance",
            "Transaction fee payer balance",
        )
    }
}

pub struct MerkleRootRelayer {
    api_provider: ApiProviderConnection,
    eth_api: EthApi,

    proof_storage: Arc<dyn ProofStorage>,

    genesis_config: GenesisConfig,

    metrics: Metrics,
}

impl MeteredService for MerkleRootRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(prover_interface::Metrics.get_sources())
    }
}

impl MerkleRootRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
        proof_storage: Arc<dyn ProofStorage>,
    ) -> MerkleRootRelayer {
        let metrics = Metrics::new();

        MerkleRootRelayer {
            api_provider,
            eth_api,
            genesis_config,
            proof_storage,
            metrics,
        }
    }

    pub async fn run(
        mut self,
        mut blocks_rx: Receiver<GearBlock>,
        mut proof_submitter: UnboundedSender<FinalProof>,
    ) -> anyhow::Result<()> {
        log::info!("Starting relayer");

        let mut attempts = 0;

        loop {
            attempts += 1;
            let now = Instant::now();
            let res = self.run_impl(&mut blocks_rx, &mut proof_submitter).await;

            if let Err(err) = res {
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

                if common::is_transport_error_recoverable(&err) {
                    self.eth_api = self
                        .eth_api
                        .reconnect()
                        .await
                        .inspect_err(|err| {
                            log::error!("Failed to reconnect to Ethereum: {err}");
                        })
                        .map_err(|err| anyhow::anyhow!(err))?;
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

    fn authority_set_hash_changeed(block: &GearBlock) -> Option<H256> {
        block.events().iter().find_map(|event| match event {
            gclient::Event::GearEthBridge(GearEthBridgeEvent::AuthoritySetHashChanged(hash)) => {
                Some(*hash)
            }
            _ => None,
        })
    }

    async fn run_impl(
        &mut self,
        blocks_rx: &mut Receiver<GearBlock>,
        proof_submitter: &mut UnboundedSender<FinalProof>,
    ) -> anyhow::Result<()> {
        /*loop {
            let balance = self.eth_api.get_approx_balance().await?;
            self.metrics.fee_payer_balance.set(balance);

            match blocks_rx.recv().await {
                Ok(block) => {
                    if let Some(merkle_root) = Self::queue_merkle_root_changed(&block) {
                        log::info!(
                            "Queue merkle root changed in block #{}: {:?}",
                            block.number(),
                            merkle_root
                        );

                        if let Some(proof) = self.get_merkle_root_proof(&block, merkle_root) {
                            proof_submitter.send(proof);
                        }
                    }
                }

                Err(RecvError::Closed) => return Ok(()),
                Err(RecvError::Lagged(n)) => {
                    log::error!("Merkle root listener lagged behind {n} Gear blocks");
                    continue;
                }
            }

            /*            match blocks_rx.recv().await {
                Ok(block) => {
                    if Self::authority_set_hash_changeed(&block).is_some() {
                        log::info!("Authority set hash changed in block #{}", block.number());
                        if !self.sync_authority_set_completely(blocks_rx).await? {
                            log::info!("Gear block listener connection closed during authority set sync, exiting");
                            return Ok(());
                        }

                        self.eras.process(&self.proof_storage).await?;
                    }

                    if let Some(merkle_root) = Self::queue_merkle_root_changed(&block) {
                        log::info!(
                            "Queue merkle root changed in block #{}: {:?}",
                            block.number(),
                            merkle_root
                        );

                        if let Some(proof) = self.get_merkle_root_proof(block, merkle_root).await? {
                            if proof_submitter.send(proof).is_err() {
                                log::warn!("Proof submitter channel is closed, exiting...");
                                return Ok(());
                            }
                        }
                    }
                }

                Err(RecvError::Closed) => {
                    log::warn!("Gear block listener connection closed");
                    return Ok(());
                }

                Err(RecvError::Lagged(n)) => {
                    log::error!(
                        "Gear block listener lagged behind {n} blocks, skipping some blocks"
                    );
                    continue;
                }
            }*/
        }*/

        Ok(())
    }

    async fn process(
        &mut self,
        submitter: &mut SubmitterIo,
        blocks_rx: &mut Receiver<GearBlock>,
        prover_tx: &mut UnboundedSender<prover::Request>,
        prover_rx: &mut UnboundedReceiver<prover::Response>,
        authority_sync_rx: &mut UnboundedReceiver<SealedNotFinalizedEra>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                block = blocks_rx.recv() => {
                    match block {
                        Ok(block) => {
                            if let Some(merkle_root) = Self::queue_merkle_root_changed(&block) {
                                if prover_tx.send(prover::Request {
                                    block_number: block.number(),
                                    block_hash: block.hash(),
                                    merkle_root
                                }).is_err() {
                                    log::warn!("Finality prover connection closed, exiting");
                                    return Ok(());
                                }

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

                response = prover_rx.recv() => {
                    let Some(response) = response else {
                       log::warn!("Finality prover connection closed, exiting");
                       return Ok(());
                    };

                    if !submitter.submit_merkle_root(response.block_number, response.proof) {
                        log::warn!("Proof submitter connection closed, exiting");
                        return Ok(());
                    }
                }

                response = authority_sync_rx.recv() => {
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
            }
        }
    }

    async fn sync_authority_set_completely(
        &mut self,
        blocks_rx: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<bool> {
        log::info!("Syncing authority set");

        loop {
            let sync_steps = match blocks_rx.recv().await {
                Ok(block) => self.sync_authority_set(&block).await?,

                Err(RecvError::Closed) => {
                    log::warn!("Gear block listener connection closed");
                    return Ok(false);
                }

                Err(RecvError::Lagged(n)) => {
                    log::warn!(
                        "Gear block listener lagged behind {n} blocks, skipping some blocks"
                    );
                    continue;
                }
            };

            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {sync_steps} authority sets");
            }
        }

        log::info!("Authority set is in sync");

        Ok(true)
    }

    async fn sync_authority_set(&mut self, block: &GearBlock) -> anyhow::Result<SyncStepCount> {
        let gear_api = self.api_provider.client();

        let finalized_head = block.hash();
        let latest_authority_set_id = gear_api.authority_set_id(finalized_head).await?;

        self.metrics
            .latest_observed_gear_era
            .set(latest_authority_set_id as i64);

        let latest_proven_authority_set_id = self.proof_storage.get_latest_authority_set_id().await;

        if let Some(&latest_proven) = latest_proven_authority_set_id.as_ref() {
            self.metrics.latest_proven_era.set(latest_proven as i64);
        }

        sync_authority_set_id(
            &gear_api,
            &self.proof_storage,
            self.genesis_config,
            latest_authority_set_id,
            latest_proven_authority_set_id,
        )
        .await
    }

    async fn get_merkle_root_proof(
        &mut self,
        block: GearBlock,
        merkle_root: H256,
    ) -> anyhow::Result<Option<FinalProof>> {
        log::info!("Submitting merkle root to ethereum");

        let gear_api = self.api_provider.client();

        let finalized_head = block.hash();
        let finalized_block_number = block.number();

        if merkle_root.is_zero() {
            log::info!("Message queue at block #{finalized_block_number} is empty. Skipping");
            return Ok(None);
        }

        log::info!(
            "Proving merkle root(0x{}) presence in block #{}",
            hex::encode(merkle_root.as_bytes()),
            finalized_block_number,
        );

        let authority_set_id = gear_api.signed_by_authority_set_id(finalized_head).await?;
        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)
            .await?;

        let proof = prover_interface::prove_final(
            &gear_api,
            inner_proof,
            self.genesis_config,
            finalized_head,
        )
        .await?;

        Ok(Some(proof))
    }
}

pub mod authority_set_sync;
pub mod prover;
pub mod submitter;
