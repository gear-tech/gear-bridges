use futures::StreamExt;
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;

use gear_rpc_client::{dto, GearApi};
use prometheus::IntGauge;
use utils_prometheus::impl_metered_service;

impl_metered_service! {
    pub(crate) struct Metrics {
        latest_stored_finality_proof: IntGauge = IntGauge::new(
            "kill_switch_latest_stored_finality_proof",
            "Latest stored finality proof",
        ),
    }
}

#[derive(Encode, Decode)]
pub struct BlockFinalityProofWithHash {
    pub hash: H256,
    pub proof: dto::BlockFinalityProof,
}

pub struct BlockFinalityArchiver {
    gear_api: GearApi,
    storage: sled::Db,
    metrics: Metrics,
}

impl BlockFinalityArchiver {
    pub fn new(gear_api: GearApi, storage: sled::Db, metrics: Metrics) -> Self {
        Self {
            gear_api,
            storage,
            metrics,
        }
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(err) = self.main_loop().await {
                log::error!(
                    "resubscribing to justifications subscription stream after an error {err:#?}"
                );
            } else {
                log::info!("justifications subscription stream closed, exiting");
                break;
            }
        }
    }

    pub async fn main_loop(&mut self) -> anyhow::Result<()> {
        let mut stream = self.gear_api.subscribe_grandpa_justifications().await?;

        loop {
            let justification = stream
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("justifications stream ended unexpectedly"))??;
            let block_number = justification.commit.target_number;

            log::debug!(
                "received GRANDPA justification: #{}, {} , {}",
                justification.commit.target_number,
                justification.commit.target_hash,
                justification.round
            );

            let (hash, finality_proof) =
                self.gear_api.produce_finality_proof(justification).await?;

            self.storage.insert(
                block_number.to_be_bytes(),
                BlockFinalityProofWithHash {
                    hash,
                    proof: finality_proof,
                }
                .encode(),
            )?;
            self.metrics
                .latest_stored_finality_proof
                .set(block_number.into());

            self.storage.flush_async().await?;
        }
    }
}
