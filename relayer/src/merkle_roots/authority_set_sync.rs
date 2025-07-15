use crate::{
    common::{sync_authority_set_id, SyncStepCount},
    message_relayer::{
        common::gear::block_listener::GearBlock, eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};
use ethereum_client::EthApi;
use futures::executor::block_on;
use gclient::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::H256;
use prometheus::IntGauge;
use prover::proving::GenesisConfig;
use std::{sync::Arc, time::Instant};
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct AuthoritySetSyncIo {
    response: UnboundedReceiver<SealedNotFinalizedEra>,
    requests: UnboundedSender<GearBlock>,
}

impl AuthoritySetSyncIo {
    pub fn new(
        response: UnboundedReceiver<SealedNotFinalizedEra>,
        requests: UnboundedSender<GearBlock>,
    ) -> Self {
        Self { response, requests }
    }

    pub async fn recv(&mut self) -> Option<SealedNotFinalizedEra> {
        self.response.recv().await
    }

    pub fn synchronize(&self, block: GearBlock) -> bool {
        self.requests.send(block).is_ok()
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

/// Authority set sync task which is responsible for synchronizing
/// authority set in proof-storage and generating new proofs.
///
/// Once proof is generated it is sent to merkle root relayer for further
/// processing.
pub struct AuthoritySetSync {
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
    eras: Eras,

    metrics: Metrics,
}

impl MeteredService for AuthoritySetSync {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(self.eras.get_sources())
    }
}

impl AuthoritySetSync {
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        proof_storage: Arc<dyn ProofStorage>,
        last_sealed: Option<u64>,
        genesis_config: GenesisConfig,
    ) -> Self {
        let eras = Eras::new(last_sealed, api_provider.clone(), eth_api, genesis_config)
            .await
            .unwrap_or_else(|err| panic!("Error while creating era storage: {err}"));
        Self {
            api_provider,
            proof_storage,
            genesis_config,
            eras,

            metrics: Metrics::new(),
        }
    }

    pub fn run(mut self, mut blocks: Receiver<GearBlock>) -> AuthoritySetSyncIo {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = AuthoritySetSyncIo::new(rx, req_tx);

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                loop {
                    if let Err(err) = self.process(&mut blocks, &mut req_rx, &tx).await {
                        log::error!("Authority set sync task failed: {err}");

                        match self.api_provider.reconnect().await {
                            Ok(_) => {
                                log::info!("Reconnected to Gear API, resuming authority set sync");
                                continue;
                            }
                            Err(err) => {
                                log::error!("Failed to reconnect to Gear API: {err}");
                                return;
                            }
                        }
                    }
                }
            })
        });

        io
    }

    fn authority_set_hash_changeed(block: &GearBlock) -> Option<H256> {
        block.events().iter().find_map(|event| match event {
            gclient::Event::GearEthBridge(GearEthBridgeEvent::AuthoritySetHashChanged(hash)) => {
                Some(*hash)
            }
            _ => None,
        })
    }

    async fn process(
        &mut self,
        blocks: &mut Receiver<GearBlock>,
        force_sync: &mut UnboundedReceiver<GearBlock>,
        sealed_eras: &UnboundedSender<SealedNotFinalizedEra>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                sync = force_sync.recv() => {
                    match sync {
                        Some(block) => {
                            log::info!("Force authority set sync for authority set in block #{}", block.number());

                            if !self.sync_authority_set_completely(&block, blocks).await? {
                                return Ok(());
                            }

                            self.eras.process(&self.proof_storage).await?;

                            while let Some(sealed) = self.eras.sealed_not_finalized.pop() {
                                if sealed_eras.send(sealed).is_err() {
                                    return Ok(());
                                }
                            }
                        }

                        None => return Ok(())
                    }
                }
                block = blocks.recv() => {
                    match block {
                        Ok(block) => {
                            if Self::authority_set_hash_changeed(&block).is_some() {
                                if !self.sync_authority_set_completely(&block, blocks).await? {
                                    return Ok(());
                                }

                                self.eras.process(&self.proof_storage).await?;

                                while let Some(sealed) = self.eras.sealed_not_finalized.pop() {
                                    if sealed_eras.send(sealed).is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }

                        Err(RecvError::Lagged(n)) => {
                            log::error!(
                                "Gear block listener lagged behind {n} blocks, skipping some blocks"
                            );
                            continue;
                        }

                        Err(RecvError::Closed) => {
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
    ) -> anyhow::Result<bool> {
        if self.sync_authority_set(initial_block).await? == 0 {
            log::info!(
                "Authority set is already in sync at block #{}",
                initial_block.number()
            );
            return Ok(false);
        }

        log::info!("Syncing authority set");
        loop {
            let sync_steps = match blocks.recv().await {
                Ok(block) => self.sync_authority_set(&block).await?,

                Err(RecvError::Closed) => {
                    log::warn!("Gear block listener connection closed");
                    return Ok(false);
                }

                Err(RecvError::Lagged(n)) => {
                    log::error!(
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
}

struct Eras {
    last_sealed: u64,
    sealed_not_finalized: Vec<SealedNotFinalizedEra>,

    api_provider: ApiProviderConnection,
    eth_api: EthApi,

    genesis_config: GenesisConfig,

    metrics: EraMetrics,
}

#[derive(Clone)]
pub struct SealedNotFinalizedEra {
    pub era: u64,
    pub merkle_root_block: u32,
    pub proof: FinalProof,
}

impl MeteredService for Eras {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct EraMetrics {
        sealed_not_finalized_count: IntGauge = IntGauge::new(
            "sealed_not_finalized_count",
            "Amount of eras that have been sealed but tx is not yet finalized by ethereum",
        ),
        last_sealed_era: IntGauge = IntGauge::new("last_sealed_era", "Latest era that have been sealed"),
    }
}

impl Eras {
    pub async fn new(
        last_sealed: Option<u64>,
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
    ) -> anyhow::Result<Self> {
        let last_sealed = if let Some(l) = last_sealed {
            l
        } else {
            let gear_api = api_provider.client();
            let latest = gear_api.latest_finalized_block().await?;
            let set_id = gear_api.authority_set_id(latest).await?;
            set_id.max(2) - 1
        };

        let metrics = EraMetrics::new();
        metrics.sealed_not_finalized_count.set(0);
        metrics.last_sealed_era.set(last_sealed as i64);

        Ok(Self {
            last_sealed,
            sealed_not_finalized: vec![],
            api_provider,
            eth_api,

            genesis_config,

            metrics,
        })
    }

    pub async fn process(&mut self, proof_storage: &Arc<dyn ProofStorage>) -> anyhow::Result<()> {
        log::info!("Processing eras");

        self.try_seal(proof_storage).await?;
        log::info!("Eras processed");

        Ok(())
    }

    async fn try_seal(&mut self, proof_storage: &Arc<dyn ProofStorage>) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        let latest = gear_api.latest_finalized_block().await?;
        let current_era = gear_api.signed_by_authority_set_id(latest).await?;

        while self.last_sealed + 2 <= current_era {
            log::info!("Sealing era #{}", self.last_sealed + 1);
            self.seal_era(self.last_sealed + 1, proof_storage).await?;
            log::info!("Sealed era #{}", self.last_sealed + 1);

            self.last_sealed += 1;

            self.metrics.last_sealed_era.inc();
        }

        Ok(())
    }

    async fn seal_era(
        &mut self,
        authority_set_id: u64,
        proof_storage: &Arc<dyn ProofStorage>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        let block = gear_api.find_era_first_block(authority_set_id + 1).await?;
        let block_number = gear_api.block_hash_to_number(block).await?;

        let queue_merkle_root = gear_api.fetch_queue_merkle_root(block).await?;
        if queue_merkle_root.is_zero() {
            log::info!("Message queue at block #{block_number} is empty. Skipping sealing");
            return Ok(());
        }

        let root_exists = self
            .eth_api
            .read_finalized_merkle_root(block_number)
            .await?
            .is_some();

        if root_exists {
            log::info!("Merkle root for era #{authority_set_id} is already submitted",);
            return Ok(());
        }

        let inner_proof = proof_storage
            .get_proof_for_authority_set_id(authority_set_id)
            .await?;

        let instant = Instant::now();
        let proof =
            prover_interface::prove_final(&gear_api, inner_proof, self.genesis_config, block)
                .await?;
        let elapsed_proof = instant.elapsed();
        log::info!("prover_interface::prove_final took {elapsed_proof:?} for block_number = #{block_number}, authority_set_id = #{authority_set_id}");

        assert_eq!(
            proof.block_number, block_number,
            "It was expected that prover_interface::prove_final 
            will not change the block number for the proof 
            in the case of the first block in the era"
        );

        self.sealed_not_finalized.push(SealedNotFinalizedEra {
            era: authority_set_id,
            merkle_root_block: block_number,
            proof,
        });

        self.metrics.sealed_not_finalized_count.inc();

        Ok(())
    }
}
