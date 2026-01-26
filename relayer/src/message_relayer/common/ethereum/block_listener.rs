use crate::{
    common,
    message_relayer::common::{
        ethereum::block_storage::UnprocessedBlockStorage, EthereumBlockNumber,
    },
};
use ethereum_client::PollingEthApi;
use prometheus::IntGauge;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct BlockListener {
    eth_api: PollingEthApi,
    from_block: u64,
    storage: Arc<dyn UnprocessedBlockStorage>,

    metrics: Metrics,
}

impl MeteredService for BlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_block: IntGauge = IntGauge::new(
            "ethereum_block_listener_latest_block",
            "Latest ethereum block discovered by listener",
        ),
    }
}

impl BlockListener {
    pub fn new(
        eth_api: PollingEthApi,
        from_block: u64,
        storage: Arc<dyn UnprocessedBlockStorage>,
    ) -> Self {
        Self {
            eth_api,
            from_block,
            storage,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self) -> UnboundedReceiver<EthereumBlockNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::spawn(self.task(sender));

        receiver
    }

    async fn run_inner(&self, sender: &UnboundedSender<EthereumBlockNumber>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);

        // Fetch unprocessed blocks in background
        let storage = self.storage.clone();
        let sender_clone = sender.clone();
        tokio::spawn(async move {
            match storage.unprocessed_blocks().await {
                Ok(blocks) => {
                    for block in blocks {
                        if let Err(e) = sender_clone.send(EthereumBlockNumber(block)) {
                            log::error!("Failed to send unprocessed block {block}: {e}");
                            break;
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to fetch unprocessed blocks: {e}");
                }
            }
        });

        loop {
            let latest = self.eth_api.finalized_block().await?.header.number;
            if latest >= current_block {
                for block in current_block..=latest {
                    if let Err(e) = self.storage.add_block(block).await {
                        log::error!("Failed to add block {block} to storage: {e}");
                    }
                    sender.send(EthereumBlockNumber(block))?;
                }

                current_block = latest + 1;

                self.metrics.latest_block.set(latest as i64);
            } else {
                tokio::time::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2).await;
            }
        }
    }

    async fn task(mut self, sender: UnboundedSender<EthereumBlockNumber>) {
        loop {
            let result = self.run_inner(&sender).await;
            let Err(e) = result else {
                continue;
            };

            log::error!("Ethereum block listener failed: {e:?}");
            if !common::is_transport_error_recoverable(&e) {
                log::error!("Non recoverable error, exiting.");
                return;
            }

            tokio::time::sleep(Duration::from_secs(30)).await;

            self.eth_api = match self.eth_api.reconnect().await {
                Ok(api) => api,
                Err(e) => {
                    log::error!("Failed to reconnect to Ethereum API: {e}");
                    return;
                }
            }
        }
    }
}
