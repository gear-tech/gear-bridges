use std::{
    path::Path,
    process,
    time::{Duration, Instant},
};

use block_finality_archiver::BlockFinalityProofWithHash;
use ethereum_client::{EthApi, MerkleRootEntry, TxHash, TxStatus};
use gear_rpc_client::GearApi;
use parity_scale_codec::Decode;
use prover::proving::GenesisConfig;

use crate::{
    common::{submit_proof_to_ethereum, sync_authority_set_id},
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

mod block_finality_archiver;

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(12);

enum State {
    Normal,
    // Before exit we need to wait for the kill switch transaction to be finalized.
    WaitingForKillSwitchTxFin { tx_hash: TxHash, proof: FinalProof },
}

pub struct KillSwitchRelayer {
    gear_api: GearApi,
    eth_api: EthApi,
    genesis_config: GenesisConfig,
    proof_storage: Box<dyn ProofStorage>,

    // Next eth block to process.
    from_eth_block: Option<u64>,
    state: State,
    block_finality_storage: sled::Db,
}

impl KillSwitchRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
        proof_storage: Box<dyn ProofStorage>,
        from_eth_block: Option<u64>,
        block_finality_storage_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            gear_api,
            eth_api,
            genesis_config,
            proof_storage,
            from_eth_block,
            state: State::Normal,
            block_finality_storage: sled::open(block_finality_storage_path)
                .expect("DB not corrupted"),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.spawn_block_finality_archiver()?;

        log::info!("Starting kill switch relayer");
        loop {
            let now = Instant::now();
            let res = match &self.state {
                State::Normal => self.main_loop().await,
                State::WaitingForKillSwitchTxFin { tx_hash, .. } => {
                    self.check_kill_switch_tx_finalized(*tx_hash).await
                }
            };

            if let Err(err) = res {
                log::error!("{}", err);
            }

            let main_loop_duration = now.elapsed();
            if main_loop_duration < MIN_MAIN_LOOP_DURATION {
                tokio::time::sleep(MIN_MAIN_LOOP_DURATION - main_loop_duration).await;
            }
        }
    }

    fn spawn_block_finality_archiver(&self) -> anyhow::Result<()> {
        log::info!("Spawning block finality archiver");
        let mut block_finality_saver = block_finality_archiver::BlockFinalityArchiver::new(
            self.gear_api.clone(),
            self.block_finality_storage.clone(),
        );
        tokio::spawn(async move {
            block_finality_saver.run().await;
        });

        Ok(())
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

        let last_finalized_block = self.eth_api.finalized_block_number().await?;

        // Set the initial value for `from_eth_block` if it's not set yet.
        if self.from_eth_block.is_none() {
            self.from_eth_block = Some(last_finalized_block);
        }

        let from_eth_block = self.from_eth_block.expect("should be set above");
        if last_finalized_block < from_eth_block {
            log::info!(
                "No new eth block, skipping.. last_processed_eth_block={}, last_finalized_block={}",
                from_eth_block.saturating_sub(1),
                last_finalized_block,
            );
            return Ok(());
        }

        let events = self
            .eth_api
            .fetch_merkle_roots_in_range(from_eth_block, last_finalized_block)
            .await?;

        for event in events {
            if !self.compare_merkle_roots(&event).await? {
                // Okay, we have a mismatch,
                // that means for some reason the proof with incorrect merkle root was submitted to relayer contract.
                // We need to generate the correct proof and submit it to the relayer contract.

                let Some(block_finality) = self
                    .get_block_finality_from_storage(event.block_number)
                    .await?
                else {
                    log::error!(
                        "Block finality proof not found for block #{}",
                        event.block_number
                    );
                    continue;
                };

                log::info!("Proving merkle root presence");
                let proof = self
                    .generate_proof(event.block_number, block_finality)
                    .await?;
                log::info!("Proven merkle root presence");

                log::info!("Submitting new proof to ethereum");
                let tx_hash = submit_proof_to_ethereum(&self.eth_api, proof.clone()).await?;
                log::info!("New proof submitted to ethereum, tx hash: 0x{:X?}", &tx_hash);

                // Resubmitting the correct proof instead of the incorrect one
                // will trigger the emergency stop condition (i.e. the kill switch) in relayer contract.
                // After that, there's no point in continuing because the relayer will be stopped/in emergency mode.
                // Though, we need to wait for the kill switch transaction to be finalized.
                self.state = State::WaitingForKillSwitchTxFin { tx_hash, proof };
                return Ok(());
            }
        }

        // After processing all events, `last_finalized_block` is the last block we've processed.
        // So, we need to increment it by 1 to set the next block to process.
        self.from_eth_block = Some(last_finalized_block.saturating_add(1));

        Ok(())
    }

    async fn check_kill_switch_tx_finalized(&mut self, tx_hash: TxHash) -> anyhow::Result<()> {
        log::info!("Checking for kill switch tx to be finalized");

        let tx_status = self.eth_api.get_tx_status(tx_hash).await?;

        match tx_status {
            TxStatus::Finalized => {
                log::info!("Kill switch tx finalized, exiting ..");
                process::exit(0);
            }
            TxStatus::Pending => (),
            TxStatus::Failed => {
                log::warn!("Re-trying kill switch tx #{} finalization", tx_hash);

                let State::WaitingForKillSwitchTxFin { tx_hash, proof } = &mut self.state else {
                    unreachable!("Invalid state");
                };

                let new_tx_hash = submit_proof_to_ethereum(&self.eth_api, proof.clone()).await?;
                // Update hash of the new kill switch transaction
                *tx_hash = new_tx_hash;
            }
        }

        Ok(())
    }

    async fn get_block_finality_from_storage(
        &mut self,
        block_number: u64,
    ) -> anyhow::Result<Option<BlockFinalityProofWithHash>> {
        let key_bytes = block_number.to_be_bytes();

        let value_bytes = match self.block_finality_storage.get(key_bytes)? {
            Some(bytes) => bytes,
            None => {
                // Block finality proof generated for series of block,
                // for example:
                //  ```text
                //          Round 1: finality proof for block #2 (transitively for 2,1 blocks)
                //           |
                //           |           Round 2: finality proof for block #5 (transitively for 5,4,3.. blocks)
                //           |           |
                //           v           v
                //      [1] [2] [3] [4] [5] [6] [7] [8] [9] [10]
                //  ```
                // Some if we don't have the proof for block #3 or #4, we can try to get the proof for block #5.
                let res = self.block_finality_storage.get_gt(key_bytes)?;
                let Some((_key, val)) = res else {
                    return Ok(None);
                };
                val
            }
        };

        Ok(Some(BlockFinalityProofWithHash::decode(
            &mut &value_bytes[..],
        )?))
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
                "Merkle root mismatch for block #{}, expected: {}, got: {}",
                event.block_number,
                merkle_root,
                event.merkle_root,
            );
        }

        Ok(is_matches)
    }

    async fn generate_proof(
        &self,
        block_number: u64,
        BlockFinalityProofWithHash {
            hash: block_hash,
            proof: block_finality,
        }: BlockFinalityProofWithHash,
    ) -> anyhow::Result<FinalProof> {
        log::info!(
            "Proving merkle root presence in block #{} with hash {}",
            block_number,
            block_hash,
        );

        let authority_set_id = self.gear_api.signed_by_authority_set_id(block_hash).await?;
        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)?;

        prover_interface::prove_final_with_block_finality(
            &self.gear_api,
            inner_proof,
            self.genesis_config,
            (block_hash, block_finality),
        )
        .await
    }
}
