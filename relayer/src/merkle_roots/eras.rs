use crate::{
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};
use ethereum_client::EthApi;
use futures::executor::block_on;
use prover::proving::GenesisConfig;
use std::{sync::Arc, time::Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct SealedNotFinalizedEra {
    pub era: u64,
    pub merkle_root_block: u32,
    pub proof: FinalProof,
}

pub struct Eras {
    last_sealed: u64,
    sealed_not_finalized: Vec<SealedNotFinalizedEra>,

    api_provider: ApiProviderConnection,
    eth_api: EthApi,

    genesis_config: GenesisConfig,

    count_thread: Option<usize>,
}

impl Eras {
    pub async fn new(
        last_sealed: Option<u64>,
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        genesis_config: GenesisConfig,

        count_thread: Option<usize>,
    ) -> anyhow::Result<Self> {
        let last_sealed = if let Some(l) = last_sealed {
            l
        } else {
            let gear_api = api_provider.client();
            let latest = gear_api.latest_finalized_block().await?;
            let set_id = gear_api.authority_set_id(latest).await?;
            set_id.max(2) - 1
        };

        Ok(Self {
            last_sealed,
            sealed_not_finalized: vec![],
            api_provider,
            eth_api,

            genesis_config,
            count_thread,
        })
    }

    pub fn seal(
        mut self,
        proof_storage: Arc<dyn ProofStorage>,
    ) -> UnboundedReceiver<SealedNotFinalizedEra> {
        let (tx, rx) = unbounded_channel();

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                if let Err(err) = self.try_seal(&proof_storage, &tx).await {
                    log::error!("Error while sealing eras: {err}");
                    return;
                }
                log::info!("Sealed {} era(s)", self.sealed_not_finalized.len());
            })
        });

        rx
    }

    async fn try_seal(
        &mut self,
        proof_storage: &Arc<dyn ProofStorage>,
        responses: &UnboundedSender<SealedNotFinalizedEra>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        let latest = gear_api.latest_finalized_block().await?;
        let current_era = gear_api.signed_by_authority_set_id(latest).await?;

        if self.last_sealed + 2 <= current_era {
            log::info!(
                "Last sealed era: {}, current era: {}, eras to seal: {}",
                self.last_sealed,
                current_era,
                current_era - self.last_sealed - 1
            );
        } else {
            log::info!(
                "No new eras to seal. Last sealed era: {}, current era: {}",
                self.last_sealed,
                current_era
            );
            return Ok(());
        }

        while self.last_sealed + 2 <= current_era {
            log::info!("Sealing era #{}", self.last_sealed + 1);
            self.seal_era(self.last_sealed + 1, proof_storage, responses)
                .await?;
            log::info!("Sealed era #{}", self.last_sealed + 1);

            self.last_sealed += 1;
        }

        Ok(())
    }

    async fn seal_era(
        &mut self,
        authority_set_id: u64,
        proof_storage: &Arc<dyn ProofStorage>,
        response: &UnboundedSender<SealedNotFinalizedEra>,
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
        let proof = prover_interface::prove_final(
            &gear_api,
            inner_proof,
            self.genesis_config,
            block,
            self.count_thread,
        )
        .await?;
        let elapsed_proof = instant.elapsed();
        log::info!("prover_interface::prove_final took {elapsed_proof:?} for block_number = #{block_number}, authority_set_id = #{authority_set_id}");

        assert_eq!(
            proof.block_number, block_number,
            "It was expected that prover_interface::prove_final 
            will not change the block number for the proof 
            in the case of the first block in the era"
        );

        response
            .send(SealedNotFinalizedEra {
                era: authority_set_id,
                merkle_root_block: block_number,
                proof,
            })
            .map_err(|_| anyhow::anyhow!("Failed to send sealed not finalized era"))?;

        Ok(())
    }
}
