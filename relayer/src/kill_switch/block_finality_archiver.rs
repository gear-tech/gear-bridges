use futures::StreamExt;
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;

use gear_rpc_client::dto;
use prometheus::IntGauge;
use utils_prometheus::impl_metered_service;

use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

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
    api_provider: ApiProviderConnection,
    storage: sled::Db,
    metrics: Metrics,
}

impl BlockFinalityArchiver {
    pub fn new(api_provider: ApiProviderConnection, storage: sled::Db, metrics: Metrics) -> Self {
        Self {
            api_provider,
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
                match self.api_provider.reconnect().await {
                    Ok(()) => {
                        log::info!("Gear block listener reconnected");
                    }
                    Err(err) => {
                        log::error!("Gear block listener unable to reconnect: {err}");
                        return;
                    }
                }
            } else {
                log::info!("justifications subscription stream closed, exiting");
                break;
            }
        }
    }

    pub async fn main_loop(&mut self) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();

        let mut stream = gear_api.subscribe_grandpa_justifications().await?;

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

            let (hash, finality_proof) = gear_api.produce_finality_proof(justification).await?;

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
