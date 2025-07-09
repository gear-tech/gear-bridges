use std::sync::Arc;

use crate::{
    message_relayer::{
        common::gear::block_listener::GearBlock, eth_to_gear::api_provider::ApiProviderConnection,
    },
    proof_storage::{self, ProofStorage},
};
use ethereum_client::EthApi;
use prometheus::IntGauge;
use prover::proving::GenesisConfig;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver, Sender},
    mpsc::UnboundedReceiver,
};
use utils_prometheus::impl_metered_service;

use super::SealedNotFinalizedEra;

impl_metered_service!(
    struct Metrics {
        sealed_not_finalized_count: IntGauge = IntGauge::new(
            "sealed_not_finalized_count",
            "Amount of eras that have been sealed but tx is not yet finalized by ethereum",
        ),
        last_sealed_era: IntGauge = IntGauge::new("last_sealed_era", "Latest era that have been sealed")
    }
);

pub struct Eras {
    last_sealed: u64,
    sealed_not_finalized: Vec<SealedNotFinalizedEra>,

    api_provider: ApiProviderConnection,
    eth_api: EthApi,

    proof_storage: Arc<dyn ProofStorage>,

    genesis_config: GenesisConfig,

    metrics: Metrics,
}

impl Eras {
    pub async fn new(
        last_sealed: Option<u64>,
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        proof_storage: Arc<dyn ProofStorage>,
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

        let metrics = Metrics::new();
        metrics.sealed_not_finalized_count.set(0);
        metrics.last_sealed_era.set(last_sealed as i64);

        Ok(Self {
            last_sealed,
            sealed_not_finalized: vec![],
            api_provider,
            eth_api,

            proof_storage,

            genesis_config,

            metrics,
        })
    }

    pub async fn process(
        &mut self,
        blocks: &mut Sender<GearBlock>,
        sync: &mut UnboundedReceiver<u64>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();

        while let Some(authority_set_id) = sync.recv().await {}

        Ok(())
    }

    async fn try_seal(
        &mut self,
        current_era: u64,
        blocks: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<()> {
        while self.last_sealed + 2 <= current_era {
            log::info!("Sealing era #{}", self.last_sealed + 1);

            log::info!("Sealed era #{}", self.last_sealed + 1);

            self.last_sealed += 1;
            self.metrics.last_sealed_era.inc();
        }

        Ok(())
    }

    async fn seal_era(&mut self, authority_set_id: u64) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();

        let block = gear_api.find_era_first_block(authority_set_id).await?;
        let block_number = gear_api.block_hash_to_number(block).await?;

        let queue_merkle_root = gear_api.fetch_queue_merkle_root(block_number).await?;

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

        let inner_proof = self
            .proof_storage
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

        let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, proof.clone()).await?;

        self.sealed_not_finalized.push(SealedNotFinalizedEra {
            era: authority_set_id,
            merkle_root_block: block_number,
            tx_hash,
            proof,
        });

        self.metrics.sealed_not_finalized_count.inc();

        Ok(())
    }
}
