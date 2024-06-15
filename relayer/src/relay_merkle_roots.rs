use prometheus::IntGauge;

use crate::{
    metrics::{impl_metered_service, MeteredService},
    proof_storage::{FileSystemProofStorage, ProofStorage},
    prover_interface::{self, FinalProof},
    GENESIS_CONFIG,
};

use ethereum_client::{Contracts as EthApi, TxHash, TxStatus};
use gear_rpc_client::GearApi;

pub struct MerkleRootRelayer {
    gear_api: GearApi,
    eth_api: EthApi,
    proof_storage: Box<dyn ProofStorage>,
    eras: Eras,

    metrics: Metrics,
}

impl_metered_service! {
    struct Metrics {
        latest_proven_era: IntGauge,
        latest_observed_gear_era: IntGauge,
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            latest_proven_era: IntGauge::new(
                "merkle_root_relayer_latest_proven_era",
                "Latest proven era number",
            )?,
            latest_observed_gear_era: IntGauge::new(
                "merkle_root_relayer_latest_observed_gear_era",
                "Latest era number observed by relayer",
            )?,
        })
    }
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

type SyncStepCount = usize;

impl MerkleRootRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        proof_storage: Box<dyn ProofStorage>,
    ) -> MerkleRootRelayer {
        let eras = Eras::new(None, gear_api.clone(), eth_api.clone())
            .await
            .unwrap_or_else(|err| panic!("Error while creating era storage: {}", err));

        let metrics = Metrics::new();

        MerkleRootRelayer {
            gear_api,
            eth_api,
            proof_storage,
            eras,
            metrics,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        log::info!("Starting relayer");

        loop {
            let res = self.main_loop().await;

            if let Err(err) = res {
                log::error!("{}", err);
            }
        }
    }

    async fn main_loop(&mut self) -> anyhow::Result<()> {
        log::info!("Syncing authority set id");
        loop {
            let sync_steps = self.sync_authority_set_id().await?;
            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {} authority set ids", sync_steps);
            }
        }
        log::info!("Authority set id is in sync");

        log::info!("Trying to seal eras");
        self.eras.try_seal(self.proof_storage.as_mut()).await?;
        log::info!("Eras sealed");

        log::info!("Trying to finalize eras");
        self.eras.try_finalize().await?;
        log::info!("Eras finalized");

        log::info!("Proving merkle root presense");
        let proof = self.prove_message_sent().await?;
        log::info!("Proven merkle root presense");

        log::info!("Submitting proof to ethereum");
        submit_proof_to_ethereum(&self.eth_api, proof).await?;
        log::info!("Proof submitted to ethereum");

        Ok(())
    }

    async fn sync_authority_set_id(&mut self) -> anyhow::Result<SyncStepCount> {
        let finalized_head = self.gear_api.latest_finalized_block().await.unwrap();
        let latest_authority_set_id = self
            .gear_api
            .authority_set_id(finalized_head)
            .await
            .unwrap();

        self.metrics
            .latest_observed_gear_era
            .set(latest_authority_set_id as i64);

        let latest_proven_authority_set_id = self.proof_storage.get_latest_authority_set_id();

        if let Some(&latest_proven) = latest_proven_authority_set_id.as_ref() {
            self.metrics.latest_proven_era.set(latest_proven as i64);
        }

        match latest_proven_authority_set_id {
            None => {
                let proof = prover_interface::prove_genesis(&self.gear_api).await?;
                self.proof_storage
                    .init(proof, GENESIS_CONFIG.authority_set_id)
                    .unwrap();

                Ok(1)
            }
            Some(latest_proven) if latest_proven < latest_authority_set_id => {
                let mut proof = self.proof_storage.get_proof_for_authority_set_id(latest_proven)?;

                for set_id in latest_proven..latest_authority_set_id {
                    proof = prover_interface::prove_validator_set_change(&self.gear_api, proof, set_id).await?;
                    self.proof_storage.update(proof.proof.clone(), set_id + 1)?;
                }

                let step_count = latest_authority_set_id - latest_proven;
                Ok(step_count as usize)
            }
            Some(latest_proven) if latest_proven == latest_authority_set_id => Ok(0),
            Some(latest_proven) => unreachable!(
                "Invalid state of proof storage detected: latest stored authority set id = {} but latest authority set id on VARA = {}", 
                latest_proven,
                latest_authority_set_id
            ),
        }
    }

    async fn prove_message_sent(&self) -> anyhow::Result<FinalProof> {
        let finalized_head = self.gear_api.latest_finalized_block().await?;

        let authority_set_id = self
            .gear_api
            .signed_by_authority_set_id(finalized_head)
            .await?;
        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)?;

        prover_interface::prove_final(&self.gear_api, inner_proof, finalized_head).await
    }
}

struct Eras {
    last_sealed: u64,
    sealed_not_finalized: Vec<SealedNotFinalizedEra>,

    gear_api: GearApi,
    eth_api: EthApi,

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
        sealed_not_finalized_count: IntGauge,
        last_sealed_era: IntGauge
    }
}

impl EraMetrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            sealed_not_finalized_count: IntGauge::new(
                "sealed_not_finalized_count",
                "Amount of eras that have been sealed but tx is not yet finalized by ethereum",
            )?,
            last_sealed_era: IntGauge::new("last_sealed_era", "Latest era that have been sealed")?,
        })
    }
}

impl Eras {
    pub async fn new(
        last_sealed: Option<u64>,
        gear_api: GearApi,
        eth_api: EthApi,
    ) -> anyhow::Result<Self> {
        let last_sealed = if let Some(l) = last_sealed {
            l
        } else {
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
            gear_api,
            eth_api,

            metrics,
        })
    }

    pub async fn try_seal(&mut self, proof_storage: &dyn ProofStorage) -> anyhow::Result<()> {
        let latest = self.gear_api.latest_finalized_block().await?;
        let current_era = self.gear_api.signed_by_authority_set_id(latest).await?;

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
        let block = self
            .gear_api
            .find_era_first_block(authority_set_id + 1)
            .await?;
        let inner_proof = proof_storage.get_proof_for_authority_set_id(authority_set_id)?;
        let proof = prover_interface::prove_final(&self.gear_api, inner_proof, block).await?;

        let block_number = self.gear_api.block_hash_to_number(block).await?;

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

        let tx_hash = submit_proof_to_ethereum(&self.eth_api, proof.clone()).await?;

        self.sealed_not_finalized.push(SealedNotFinalizedEra {
            era: authority_set_id,
            merkle_root_block: block_number,
            tx_hash,
            proof,
        });

        self.metrics.sealed_not_finalized_count.inc();

        Ok(())
    }

    pub async fn try_finalize(&mut self) -> anyhow::Result<()> {
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

                self.tx_hash = submit_proof_to_ethereum(eth_api, self.proof.clone()).await?;
                Ok(false)
            }
        }
    }
}

async fn submit_proof_to_ethereum(eth_api: &EthApi, proof: FinalProof) -> anyhow::Result<TxHash> {
    log::info!(
        "Submitting merkle root {} at gear block {} to ethereum",
        hex::encode(proof.merkle_root),
        proof.block_number
    );

    let tx_hash = eth_api
        .provide_merkle_root(proof.block_number, proof.merkle_root, &proof.proof[..])
        .await?;

    Ok(tx_hash)
}
