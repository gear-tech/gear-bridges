use std::{
    process,
    time::{Duration, Instant},
};

use block_finality_archiver::BlockFinalityProofWithHash;
use ethereum_client::{EthApi, MerkleRootEntry, TxHash, TxStatus};
use gear_rpc_client::GearApi;
use parity_scale_codec::Decode;
use prometheus::{Gauge, IntCounter, IntGauge};
use prover::proving::GenesisConfig;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{submit_merkle_root_to_ethereum, sync_authority_set_id, SyncStepCount},
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

mod block_finality_archiver;

const MIN_MAIN_LOOP_DURATION: Duration = Duration::from_secs(12);

impl_metered_service! {
    struct Metrics {
        latest_proven_era: IntGauge = IntGauge::new(
            "kill_switch_latest_proven_era",
            "Latest proven era number",
        ),
        latest_observed_gear_era: IntGauge = IntGauge::new(
            "kill_switch_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
        fee_payer_balance: Gauge = Gauge::new(
            "kill_switch_fee_payer_balance",
            "Transaction fee payer balance",
        ),
        latest_eth_block: IntGauge = IntGauge::new(
            "kill_switch_latest_eth_block",
            "Latest block number observed",
        ),
        merkle_roots_discovered_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_roots_discovered_cnt",
            "Amount of merkle root events discovered",
        ),
        merkle_root_mismatch_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_root_mismatch_cnt",
            "Amount of merkle root mismatches found",
        ),
        latest_stored_finality_proof: IntGauge = IntGauge::new(
            "kill_switch_latest_stored_finality_proof",
            "Latest stored finality proof",
        ),
        finality_proof_for_mismatched_root_not_found_cnt: IntCounter = IntCounter::new(
            "kill_switch_finality_proof_for_mismatched_root_not_found_cnt",
            "Amount of not found finality proofs",
        ),
    }
}

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

    start_from_eth_block: Option<u64>,
    state: State,
    block_finality_storage: sled::Db,

    metrics: Metrics,
}

impl MeteredService for KillSwitchRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(prover_interface::Metrics.get_sources())
    }
}

impl KillSwitchRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
        proof_storage: Box<dyn ProofStorage>,
        from_eth_block: Option<u64>,
        block_finality_storage: sled::Db,
    ) -> Self {
        Self {
            gear_api,
            eth_api,
            genesis_config,
            proof_storage,
            start_from_eth_block: from_eth_block,
            state: State::Normal,
            block_finality_storage,
            metrics: Metrics::new(),
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
            self.metrics.clone(),
        );
        tokio::spawn(async move {
            block_finality_saver.run().await;
        });

        Ok(())
    }

    async fn main_loop(&mut self) -> anyhow::Result<()> {
        let balance = self.eth_api.get_approx_balance().await?;
        self.metrics.fee_payer_balance.set(balance);

        log::info!("Syncing authority set id");
        loop {
            let sync_steps = self.sync_authority_set_id().await?;
            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {} authority set ids", sync_steps);
            }
        }

        let last_finalized_block = self.eth_api.finalized_block_number().await?;

        // Set the initial value for `from_eth_block` if it's not set yet.
        if self.start_from_eth_block.is_none() {
            self.start_from_eth_block = Some(last_finalized_block);
        }

        let start_from_eth_block = self.start_from_eth_block.expect("should be set above");
        if last_finalized_block < start_from_eth_block {
            log::info!(
                "No new eth block, skipping.. last_processed_eth_block={}, last_finalized_block={}",
                start_from_eth_block.saturating_sub(1),
                last_finalized_block,
            );
            return Ok(());
        } else {
            self.metrics
                .latest_eth_block
                .set(last_finalized_block as i64);
        }

        let events = self
            .eth_api
            .fetch_merkle_roots_in_range(start_from_eth_block, last_finalized_block)
            .await?;

        if !events.is_empty() {
            self.metrics
                .merkle_roots_discovered_cnt
                .inc_by(events.len() as u64);
        }

        for event in events {
            if !self.compare_merkle_roots(&event).await? {
                // Okay, we have a mismatch,
                // that means for some reason the proof with incorrect merkle root was submitted to relayer contract.
                // We need to generate the correct proof and submit it to the relayer contract.
                log::debug!("Got event with mismatched merkle root: {:?}", &event);

                let Some(block_finality) = self
                    .get_block_finality_from_storage(event.block_number)
                    .await?
                else {
                    log::error!(
                        "Block finality proof not found for block #{}",
                        event.block_number
                    );
                    self.metrics
                        .finality_proof_for_mismatched_root_not_found_cnt
                        .inc();
                    continue;
                };

                log::info!("Proving merkle root presence");
                let proof = self
                    .generate_proof(event.block_number, block_finality)
                    .await?;
                log::info!("Proven merkle root presence");

                log::info!("Submitting new proof to ethereum");
                let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, proof.clone()).await?;
                log::info!("New proof submitted to ethereum, tx hash: {:X?}", &tx_hash);

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
        self.start_from_eth_block = Some(last_finalized_block.saturating_add(1));

        Ok(())
    }

    async fn sync_authority_set_id(&mut self) -> anyhow::Result<SyncStepCount> {
        let finalized_head = self
            .gear_api
            .latest_finalized_block()
            .await
            .expect("should not fail");
        let latest_authority_set_id = self
            .gear_api
            .authority_set_id(finalized_head)
            .await
            .expect("should not fail");

        self.metrics
            .latest_observed_gear_era
            .set(latest_authority_set_id as i64);

        let latest_proven_authority_set_id = self.proof_storage.get_latest_authority_set_id();

        if let Some(&latest_proven) = latest_proven_authority_set_id.as_ref() {
            self.metrics.latest_proven_era.set(latest_proven as i64);
        }

        sync_authority_set_id(
            &self.gear_api,
            self.proof_storage.as_mut(),
            self.genesis_config,
            latest_authority_set_id,
            latest_proven_authority_set_id,
        )
        .await
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

                let new_tx_hash =
                    submit_merkle_root_to_ethereum(&self.eth_api, proof.clone()).await?;
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
        // NOTE: we use 32-bit BE keys for block finality storage.
        let key_bytes = (block_number as u32).to_be_bytes();

        let block_finality = self
            .block_finality_storage
            .get(key_bytes)?
            .map(|value_bytes| BlockFinalityProofWithHash::decode(&mut &value_bytes[..]))
            .inspect(|block_finality| {
                if let Ok(block_finality) = block_finality {
                    log::debug!(
                        "Block finality proof found with block hash {:X?}",
                        block_finality.hash
                    );
                }
            })
            .transpose();

        Ok(block_finality?)
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
                "Merkle root mismatch for block #{}, hash {:X?}, expected: {}, got: {}",
                event.block_number,
                block_hash,
                merkle_root,
                event.merkle_root,
            );
            self.metrics.merkle_root_mismatch_cnt.inc();
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
