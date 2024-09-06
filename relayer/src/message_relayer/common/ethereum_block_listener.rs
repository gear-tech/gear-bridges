use std::{
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use ethereum_client::EthApi;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use super::EthereumBlockNumber;

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct EthereumBlockListener {
    eth_api: EthApi,
    from_block: u64,

    metrics: Metrics,
}

impl MeteredService for EthereumBlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_block: IntGauge,
    }
}

impl Metrics {
    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            latest_block: IntGauge::new(
                "ethereum_block_listener_latest_block",
                "Latest ethereum block discovered by listener",
            )?,
        })
    }
}

impl EthereumBlockListener {
    pub fn new(eth_api: EthApi, from_block: u64) -> Self {
        Self {
            eth_api,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self) -> Receiver<EthereumBlockNumber> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    log::error!("Ethereum block listener failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(&self, sender: &Sender<EthereumBlockNumber>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);

        loop {
            let latest = self.eth_api.block_number().await?;
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
