use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use ethereum_client::EthApi;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::{AuthoritySetId, EthereumBlockNumber, GearBlockNumber, RelayedMerkleRoot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,

    metrics: Metrics,
}

impl MeteredService for MerkleRootExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_merkle_root_for_block: IntGauge = IntGauge::new(
            "merkle_root_extractor_latest_merkle_root_for_block",
            "Latest gear block present in found merkle roots",
        )
    }
}

impl MerkleRootExtractor {
    pub fn new(eth_api: EthApi, api_provider: ApiProviderConnection) -> Self {
        Self {
            eth_api,
            api_provider,

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: UnboundedReceiver<EthereumBlockNumber>,
    ) -> UnboundedReceiver<RelayedMerkleRoot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            let mut attempts = 0;

            loop {
                let res = self.run_inner(&mut blocks, &sender).await;
                if let Err(err) = res {
                    attempts += 1;
                    log::error!(
                        "Merkle root extractor failed (attempt {}/{}): {}. Retrying in {:?}...",
                        attempts,
                        MAX_RETRIES,
                        err,
                        BASE_RETRY_DELAY * 2u32.pow(attempts - 1),
                    );
                    if attempts >= MAX_RETRIES {
                        log::error!("Merkle root extractor failed {} times: {}", attempts, err);
                        break;
                    }

                    tokio::time::sleep(BASE_RETRY_DELAY * 2u32.pow(attempts - 1)).await;

                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("API provider reconnected");
                        }

                        Err(err) => {
                            log::error!("Merkle root extractor unable to reconnect: {err}");
                            return;
                        }
                    }

                    if common::is_transport_error_recoverable(&err) {
                        self.eth_api = match self.eth_api.reconnect() {
                            Ok(eth_api) => eth_api,
                            Err(err) => {
                                log::error!("Failed to reconnect to Ethereum: {}", err);
                                break;
                            }
                        };
                    } else {
                        log::error!("Merkle root extractor failed: {}", err);
                        break;
                    }
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        blocks: &mut UnboundedReceiver<EthereumBlockNumber>,
        sender: &UnboundedSender<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        loop {
            while let Some(block) = blocks.recv().await {
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

                    let block_hash = gear_api
                        .block_number_to_hash(merkle_root.block_number as u32)
                        .await?;

                    let authority_set_id =
                        AuthoritySetId(gear_api.signed_by_authority_set_id(block_hash).await?);

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
