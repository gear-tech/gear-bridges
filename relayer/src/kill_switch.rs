use std::time::{Duration, Instant};

use ethereum_client::{EthApi, MerkleRootEntry};
use gear_rpc_client::GearApi;
use prover::proving::GenesisConfig;

use crate::{
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
    relay_merkle_roots::{submit_proof_to_ethereum, sync_authority_set_id},
};

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(12);

pub struct KillSwitchRelayer {
    gear_api: GearApi,
    eth_api: EthApi,
    genesis_config: GenesisConfig,
    proof_storage: Box<dyn ProofStorage>,

    from_block: Option<u64>,
    last_processed_eth_block_num: u64,
}

impl KillSwitchRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
        proof_storage: Box<dyn ProofStorage>,
        from_block: Option<u64>,
    ) -> Self {
        Self {
            gear_api,
            eth_api,
            genesis_config,
            proof_storage,
            from_block,
            last_processed_eth_block_num: 0,
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        log::info!("Starting kill switch relayer");

        loop {
            let now = Instant::now();
            let res = self.main_loop().await;

            if let Err(err) = res {
                log::error!("{}", err);
            }

            let main_loop_duration = now.elapsed();
            if main_loop_duration < MIN_MAIN_LOOP_DURATION {
                tokio::time::sleep(MIN_MAIN_LOOP_DURATION - main_loop_duration).await;
            }
        }
    }

    async fn main_loop(&mut self) -> anyhow::Result<()> {
        log::info!("Syncing authority set id");
        loop {
            let sync_steps = sync_authority_set_id(
                &self.gear_api,
                self.proof_storage.as_mut(),
                self.genesis_config,
                None,
            )
            .await?;
            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {} authority set ids", sync_steps);
            }
        }

        let last_fin_block = self.eth_api.finalized_block_number().await?;
        if last_fin_block == self.last_processed_eth_block_num {
            log::info!("No new Eth block, skipping..");
            return Ok(());
        }

        let block_from = self.from_block.unwrap_or(last_fin_block);

        let events = self
            .eth_api
            .fetch_merkle_roots_in_range(block_from, last_fin_block)
            .await?;

        for event in events {
            if !self.compare_merkle_roots(&event).await? {
                // Okay, we have a mismatch,
                // that means for some reason the proof with incorrect merkle root was submitted to relayer contract.
                // We need to generate the correct proof and submit it to the relayer contract.

                log::info!("Proving merkle root presence");
                let proof = self.generate_proof(event.block_number).await?;
                log::info!("Proven merkle root presence");

                log::info!("Submitting new proof to ethereum");
                submit_proof_to_ethereum(&self.eth_api, proof).await?;
                log::info!("New proof submitted to ethereum");

                // Resubmitting the correct proof instead of the incorrect one
                // will trigger the emergency stop condition (i.e. the kill switch) in relayer contract.
                // After that, there's no point in continuing because the relayer will be stopped/in emergency mode.
                return Ok(());
            }
        }

        self.last_processed_eth_block_num = last_fin_block;

        Ok(())
    }

    async fn compare_merkle_roots(&self, event: &MerkleRootEntry) -> anyhow::Result<bool> {
        let block_hash = self
            .gear_api
            .block_number_to_hash(event.block_number as u32)
            .await?;
        let merkle_root = self.gear_api.fetch_queue_merkle_root(block_hash).await?;

        let is_matches = merkle_root == event.merkle_root;

        if !is_matches {
            log::info!(
                "Merkle root mismatch for block number: {}, expected: {}, got: {}",
                event.block_number,
                event.merkle_root,
                merkle_root
            );
        }

        Ok(is_matches)
    }

    async fn generate_proof(&self, block_number: u64) -> anyhow::Result<FinalProof> {
        let block_hash = self
            .gear_api
            .block_number_to_hash(block_number as u32)
            .await?;

        log::info!(
            "Proving merkle root presense in block #{} with hash {}",
            block_number,
            block_hash,
        );

        let authority_set_id = self.gear_api.signed_by_authority_set_id(block_hash).await?;
        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)?;

        prover_interface::prove_final(&self.gear_api, inner_proof, self.genesis_config, block_hash)
            .await
    }
}
