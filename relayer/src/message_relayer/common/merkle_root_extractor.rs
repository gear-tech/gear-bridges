use std::sync::mpsc::{channel, Receiver, Sender};

use ethereum_client::EthApi;
use futures::executor::block_on;
use gear_rpc_client::GearApi;
use prometheus::IntGauge;

use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::common::GearBlockNumber;

use super::{AuthoritySetId, EthereumBlockNumber, RelayedMerkleRoot};

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    gear_api: GearApi,

    metrics: Metrics,
}

impl MeteredService for MerkleRootExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_merkle_root_for_block: IntGauge
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            latest_merkle_root_for_block: IntGauge::new(
                "merkle_root_extractor_latest_merkle_root_for_block",
                "Latest gear block present in found merkle roots",
            )?,
        })
    }
}

impl MerkleRootExtractor {
    pub fn new(eth_api: EthApi, gear_api: GearApi) -> Self {
        Self {
            eth_api,
            gear_api,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<EthereumBlockNumber>) -> Receiver<RelayedMerkleRoot> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = block_on(self.run_inner(&blocks, &sender));
                if let Err(err) = res {
                    log::error!("Merkle root extractor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        blocks: &Receiver<EthereumBlockNumber>,
        sender: &Sender<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                let merkle_roots = self
                    .eth_api
                    .fetch_merkle_roots_in_range(block.0, block.0)
                    .await?;

                if !merkle_roots.is_empty() {
                    log::info!(
                        "Found {} merkle roots at block #{}",
                        merkle_roots.len(),
                        block
                    );
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
                        AuthoritySetId(self.gear_api.signed_by_authority_set_id(block_hash).await?);

                    log::info!(
                        "Found merkle root for gear block #{} and era #{}",
                        merkle_root.block_number,
                        authority_set_id
                    );

                    sender.send(RelayedMerkleRoot {
                        block: GearBlockNumber(merkle_root.block_number as u32),
                        authority_set_id,
                    })?;
                }
            }
        }
    }
}
