use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use ethereum_client::EthApi;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{common, message_relayer::common::EthereumBlockNumber};

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct BlockListener {
    eth_api: EthApi,
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
    pub fn new(eth_api: EthApi, from_block: u64) -> Self {
        Self {
            eth_api,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub async fn run(mut self) -> UnboundedReceiver<EthereumBlockNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::spawn(async move {
            let base_delay = Duration::from_secs(1);
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 5;

            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    attempts += 1;
                    log::error!(
                        "Ethereum block listener failed (attempt {}/{}): {}. Retrying in {:?}",
                        err,
                        attempts,
                        MAX_ATTEMPTS,
                        base_delay * 2u32.pow(attempts - 1),
                    );
                    if attempts >= MAX_ATTEMPTS {
                        log::error!("Maximum attempts reached, exiting...");
                        return;
                    }
                    if common::is_transport_error_recoverable(&err) {
                        tokio::time::sleep(base_delay * 2u32.pow(attempts - 1)).await;

                        self.eth_api = match self.eth_api.reconnect() {
                            Ok(api) => api,
                            Err(err) => {
                                log::error!("Failed to reconnect to Ethereum API: {}", err);
                                return;
                            }
                        }
                    }
                }
            }
        });

        receiver
    }

    async fn run_inner(&self, sender: &UnboundedSender<EthereumBlockNumber>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);

        loop {
            let latest = self.eth_api.finalized_block_number().await?;
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
