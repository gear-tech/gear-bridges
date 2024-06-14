use std::{
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::Duration,
};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;
use prometheus::IntGauge;

use crate::metrics::{impl_metered_service, MeteredService};

use super::RelayedMerkleRoot;

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct MerkleRootListener {
    eth_api: EthApi,
    gear_api: GearApi,
    from_block: u64,

    metrics: Metrics,
}

impl MeteredService for MerkleRootListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_processed_block: IntGauge,
        latest_merkle_root_for_block: IntGauge
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            latest_processed_block: IntGauge::new(
                "merkle_root_listener_latest_processed_block",
                "Latest ethereum block processed by merkle root listener",
            )?,
            latest_merkle_root_for_block: IntGauge::new(
                "merkle_root_listener_latest_merkle_root_for_block",
                "Latest gear block present in found merkle roots",
            )?,
        })
    }
}

impl MerkleRootListener {
    pub fn new(eth_api: EthApi, gear_api: GearApi, from_block: u64) -> Self {
        Self {
            eth_api,
            gear_api,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self) -> Receiver<RelayedMerkleRoot> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    log::error!("Merkle root listener failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(&self, sender: &Sender<RelayedMerkleRoot>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_processed_block.set(current_block as i64);

        loop {
            let latest = self.eth_api.block_number().await?;
            if latest >= current_block {
                log::info!("Processing ethereum blocks #{}..#{}", current_block, latest);
                let merkle_roots = self
                    .eth_api
                    .fetch_merkle_roots_in_range(current_block, latest)
                    .await?;

                if !merkle_roots.is_empty() {
                    log::info!("Found {} merkle roots", merkle_roots.len());
                }

                for merkle_root in merkle_roots {
                    self.metrics
                        .latest_merkle_root_for_block
                        .set(merkle_root.block_number as i64);

                    let block_hash = self
                        .gear_api
                        .block_number_to_hash(merkle_root.block_number as u32)
                        .await?;

                    let authority_set_id =
                        self.gear_api.signed_by_authority_set_id(block_hash).await?;

                    log::info!(
                        "Found merkle root for gear block #{} and era #{}",
                        merkle_root.block_number,
                        authority_set_id
                    );

                    sender.send(RelayedMerkleRoot {
                        gear_block: merkle_root.block_number as u32,
                        authority_set_id,
                    })?;
                }

                current_block = latest + 1;
                self.metrics.latest_processed_block.inc();
            } else {
                thread::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2)
            }
        }
    }
}
