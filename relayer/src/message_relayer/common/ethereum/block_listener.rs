use crate::{common, message_relayer::common::EthereumBlockNumber};
use ethereum_client::PollingEthApi;
use prometheus::IntGauge;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct BlockListener {
    eth_api: PollingEthApi,
    from_block: u64,

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
    pub fn new(eth_api: PollingEthApi, from_block: u64) -> Self {
        Self {
            eth_api,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self) -> UnboundedReceiver<EthereumBlockNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::spawn(self::task(self, sender));

        receiver
    }

    async fn run_inner(&self, sender: &UnboundedSender<EthereumBlockNumber>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);

        loop {
            let latest = self.eth_api.finalized_block().await?.header.number;
            if latest >= current_block {
                for block in current_block..=latest {
                    sender.send(EthereumBlockNumber(block))?;
                }

                current_block = latest + 1;

                self.metrics.latest_block.set(latest as i64);
            } else {
                tokio::time::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2).await;
            }
        }
    }
}

async fn task(mut this: BlockListener, sender: UnboundedSender<EthereumBlockNumber>) {
    loop {
        let result = this.run_inner(&sender).await;

        let Err(e) = result else {
            continue;
        };

        log::error!("Ethereum block listener failed: {e:?}");

        if common::is_transport_error_recoverable(&e) {
            tokio::time::sleep(Duration::from_secs(30)).await;

            this.eth_api = match this.eth_api.reconnect().await {
                Ok(api) => api,
                Err(e) => {
                    log::error!("Failed to reconnect to Ethereum API: {e}");
                    return;
                }
            }
        } else {
            return;
        }
    }
}
