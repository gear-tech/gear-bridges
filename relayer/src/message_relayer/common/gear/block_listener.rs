use prometheus::IntGauge;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{
    common::GearBlockNumber, eth_to_gear::api_provider::ApiProviderConnection,
};

const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

pub struct BlockListener {
    api_provider: ApiProviderConnection,
    from_block: u32,

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
            "gear_block_listener_latest_block",
            "Latest gear block discovered by gear block listener",
        )
    }
}

impl BlockListener {
    pub fn new(api_provider: ApiProviderConnection, from_block: u32) -> Self {
        Self {
            api_provider,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [UnboundedReceiver<GearBlockNumber>; RECEIVER_COUNT] {
        let (senders, receivers): (Vec<_>, Vec<_>) =
            (0..RECEIVER_COUNT).map(|_| unbounded_channel()).unzip();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&senders).await;
                if let Err(err) = res {
                    log::error!("Gear block listener failed: {}", err);

                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Gear block listener reconnected");
                        }
                        Err(err) => {
                            log::error!("Gear block listener unable to reconnect: {}", err);
                            return;
                        }
                    };
                }
            }
        });

        receivers
            .try_into()
            .expect("Expected Vec of correct length")
    }

    async fn run_inner(&self, senders: &[UnboundedSender<GearBlockNumber>]) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);
        let gear_api = self.api_provider.client();
        loop {
            let finalized_head = gear_api.latest_finalized_block().await?;
            let finalized_head = gear_api.block_hash_to_number(finalized_head).await?;

            if finalized_head >= current_block {
                for block in current_block..=finalized_head {
                    for sender in senders {
                        sender.send(GearBlockNumber(block))?;
                    }

                    self.metrics.latest_block.inc();
                }

                current_block = finalized_head + 1;
            } else {
                tokio::time::sleep(GEAR_BLOCK_TIME_APPROX).await;
            }
        }
    }
}
