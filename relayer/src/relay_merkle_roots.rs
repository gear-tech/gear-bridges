use std::time::{Duration, Instant};

use prometheus::{Gauge, IntGauge};

use ethereum_client::{EthApi, TxHash, TxStatus};
use prover::proving::GenesisConfig;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{
        self, submit_merkle_root_to_ethereum, sync_authority_set_id, SyncStepCount,
        BASE_RETRY_DELAY, MAX_RETRIES,
    },
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

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

    proof_storage: Box<dyn ProofStorage>,
    eras: Eras,

    latest_submitted_merkle_root: Option<SubmittedMerkleRoot>,

    genesis_config: GenesisConfig,

    metrics: Metrics,
}

struct SubmittedMerkleRoot {
    tx_hash: TxHash,
    proof: FinalProof,
    finalized: bool,
}

impl MeteredService for MerkleRootRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(self.eras.get_sources())
            .chain(prover_interface::Metrics.get_sources())
    }
}

impl MerkleRootRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        genesis_config: GenesisConfig,
        proof_storage: Box<dyn ProofStorage>,
    ) -> MerkleRootRelayer {
        let eras = Eras::new(None, api_provider.clone(), eth_api.clone(), genesis_config)
            .await
            .unwrap_or_else(|err| panic!("Error while creating era storage: {}", err));

        let metrics = Metrics::new();

        MerkleRootRelayer {
            api_provider,
            eth_api,
            genesis_config,
            proof_storage,
            latest_submitted_merkle_root: None,
            eras,
            metrics,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        log::info!("Starting relayer");

        let mut attempts = 0;

        loop {
            attempts += 1;
            let now = Instant::now();
            let res = self.main_loop().await;

            if let Err(err) = res {
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                log::error!(
                    "Main loop error (attempt {}/{}): {}. Retrying in {:?}...",
                    attempts,
                    MAX_RETRIES,
                    err,
                    delay
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
                        log::error!("Failed to reconnect to Gear API: {}", err);
                        return Err(err.context("Failed to reconnect to Gear API"));
                    }
                }

                if common::is_transport_error_recoverable(&err) {
                    self.eth_api = self
                        .eth_api
                        .reconnect()
                        .inspect_err(|err| {
                            log::error!("Failed to reconnect to Ethereum: {}", err);
                        })
                        .map_err(|err| anyhow::anyhow!(err))?;
                    self.eras.update_eth_api(self.eth_api.clone());
                }
            }

            let main_loop_duration = now.elapsed();
            if main_loop_duration < MIN_MAIN_LOOP_DURATION {
                tokio::time::sleep(MIN_MAIN_LOOP_DURATION - main_loop_duration).await;
            }
        }
    }

    async fn main_loop(&mut self) -> anyhow::Result<()> {
        let balance = self.eth_api.get_approx_balance().await?;
        self.metrics.fee_payer_balance.set(balance);

        self.sync_authority_set_completely().await?;

        self.eras.process(self.proof_storage.as_mut()).await?;

        self.submit_merkle_root().await?;

        self.try_finalize_submitted_merkle_root().await
    }

    async fn sync_authority_set_completely(&mut self) -> anyhow::Result<()> {
        log::info!("Syncing authority set");

        loop {
            let sync_steps = self.sync_authority_set().await?;
            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {} authority sets", sync_steps);
            }
        }

        log::info!("Authority set is in sync");

        Ok(())
    }

    async fn sync_authority_set(&mut self) -> anyhow::Result<SyncStepCount> {
        let gear_api = self.api_provider.client();
        let finalized_head = gear_api
            .latest_finalized_block()
            .await
            .expect("should not fail");
        let latest_authority_set_id = gear_api
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
            &gear_api,
            self.proof_storage.as_mut(),
            self.genesis_config,
            latest_authority_set_id,
            latest_proven_authority_set_id,
        )
        .await
    }

    async fn submit_merkle_root(&mut self) -> anyhow::Result<()> {
        log::info!("Submitting merkle root to ethereum");

        let gear_api = self.api_provider.client();

        let finalized_head = gear_api.latest_finalized_block().await?;
        let finalized_block_number = gear_api.block_hash_to_number(finalized_head).await?;

        let merkle_root = gear_api.fetch_queue_merkle_root(finalized_head).await?;

        if merkle_root.is_zero() {
            log::info!(
                "Message queue at block #{} is empty. Skipping",
                finalized_block_number
            );
            return Ok(());
        }

        if let Some(submitted_merkle_root) = &self.latest_submitted_merkle_root {
            if submitted_merkle_root.proof.merkle_root == merkle_root.0 {
                log::info!(
                    "Message queue at block #{} don't contain new messages. Skipping",
                    finalized_block_number
                );
                return Ok(());
            }
        }

        log::info!(
            "Proving merkle root(0x{}) presence in block #{}",
            hex::encode(merkle_root.as_bytes()),
            finalized_block_number,
        );

        let authority_set_id = gear_api.signed_by_authority_set_id(finalized_head).await?;
        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)?;

        let proof = prover_interface::prove_final(
            &gear_api,
            inner_proof,
            self.genesis_config,
            finalized_head,
        )
        .await?;

        let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, proof.clone()).await?;

        log::info!("Merkle root submitted to ethereum");

        self.latest_submitted_merkle_root = Some(SubmittedMerkleRoot {
            tx_hash,
            proof,
            finalized: false,
        });

        Ok(())
    }

    async fn try_finalize_submitted_merkle_root(&mut self) -> anyhow::Result<()> {
        let Some(submitted_merkle_root) = &mut self.latest_submitted_merkle_root else {
            return Ok(());
        };

        if submitted_merkle_root.finalized {
            return Ok(());
        }

        log::info!(
            "Trying to finalize tx containing merkle root 0x{}",
            hex::encode(submitted_merkle_root.proof.merkle_root)
        );

        let tx_status = self
            .eth_api
            .get_tx_status(submitted_merkle_root.tx_hash)
            .await?;

        match tx_status {
            TxStatus::Finalized => {
                submitted_merkle_root.finalized = true;

                log::info!(
                    "Tx containing merkle root 0x{} finalized",
                    hex::encode(submitted_merkle_root.proof.merkle_root)
                );

                Ok(())
            }
            TxStatus::Pending => Ok(()),
            TxStatus::Failed => {
                let root_exists = self
                    .eth_api
                    .read_finalized_merkle_root(submitted_merkle_root.proof.block_number)
                    .await?
                    .is_some();

                // Someone already relayed this merkle root.
                if root_exists {
                    log::info!(
                        "Merkle root 0x{} was already finalized",
                        hex::encode(submitted_merkle_root.proof.merkle_root)
                    );

                    submitted_merkle_root.finalized = true;
                    return Ok(());
                }

                log::warn!(
                    "Re-trying merkle root 0x{} sending",
                    hex::encode(submitted_merkle_root.proof.merkle_root)
                );

                submitted_merkle_root.tx_hash = submit_merkle_root_to_ethereum(
                    &self.eth_api,
                    submitted_merkle_root.proof.clone(),
                )
                .await?;

                Ok(())
            }
        }
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

struct SealedNotFinalizedEra {
    era: u64,
    merkle_root_block: u32,
    tx_hash: TxHash,
    proof: FinalProof,
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

    fn update_eth_api(&mut self, eth_api: EthApi) {
        self.eth_api = eth_api;
    }

    pub async fn process(&mut self, proof_storage: &dyn ProofStorage) -> anyhow::Result<()> {
        log::info!("Processing eras");

        self.try_seal(proof_storage).await?;
        self.try_finalize().await?;

        log::info!("Eras processed");

        Ok(())
    }

    async fn try_seal(&mut self, proof_storage: &dyn ProofStorage) -> anyhow::Result<()> {
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
        proof_storage: &dyn ProofStorage,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        let block = gear_api.find_era_first_block(authority_set_id + 1).await?;
        let inner_proof = proof_storage.get_proof_for_authority_set_id(authority_set_id)?;
        let proof =
            prover_interface::prove_final(&gear_api, inner_proof, self.genesis_config, block)
                .await?;

        let block_number = gear_api.block_hash_to_number(block).await?;

        assert_eq!(
            proof.block_number, block_number,
            "It was expected that prover_interface::prove_final 
            will not change the block number for the proof 
            in the case of the first block in the era"
        );

        let queue_merkle_root = gear_api.fetch_queue_merkle_root(block).await?;

        if queue_merkle_root.is_zero() {
            log::info!(
                "Message queue at block #{} is empty. Skipping sealing",
                block_number
            );
            return Ok(());
        }

        let root_exists = self
            .eth_api
            .read_finalized_merkle_root(block_number)
            .await?
            .is_some();

        if root_exists {
            log::info!(
                "Merkle root for era #{} is already submitted",
                authority_set_id
            );
            return Ok(());
        }

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

    async fn try_finalize(&mut self) -> anyhow::Result<()> {
        for i in (0..self.sealed_not_finalized.len()).rev() {
            if self.sealed_not_finalized[i]
                .try_finalize(&self.eth_api)
                .await?
            {
                log::info!("Era #{} finalized", self.sealed_not_finalized[i].era);
                self.sealed_not_finalized.remove(i);

                self.metrics.sealed_not_finalized_count.dec();
            } else {
                log::info!(
                    "Cannot finalize era #{} yet",
                    self.sealed_not_finalized[i].era
                );
            }
        }

        Ok(())
    }
}

impl SealedNotFinalizedEra {
    pub async fn try_finalize(&mut self, eth_api: &EthApi) -> anyhow::Result<bool> {
        let tx_status = eth_api.get_tx_status(self.tx_hash).await?;

        match tx_status {
            TxStatus::Finalized => Ok(true),
            TxStatus::Pending => Ok(false),
            TxStatus::Failed => {
                let root_exists = eth_api
                    .read_finalized_merkle_root(self.merkle_root_block)
                    .await?
                    .is_some();

                // Someone already relayed this merkle root.
                if root_exists {
                    log::info!("Era #{} is already finalized", self.era);
                    return Ok(true);
                }

                log::warn!("Re-trying era #{} finalization", self.era);

                self.tx_hash = submit_merkle_root_to_ethereum(eth_api, self.proof.clone()).await?;
                Ok(false)
            }
        }
    }
}
