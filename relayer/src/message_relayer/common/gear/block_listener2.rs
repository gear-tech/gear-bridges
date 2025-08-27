use prometheus::IntGauge;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};
use crate::message_relayer::{
    common::GearBlock, eth_to_gear::api_provider::ApiProviderConnection,
};
use subxt::config::Header;
use tokio::sync::broadcast;

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
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self {
            api_provider,
            from_block: 1,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        const CAPACITY: usize = 14_400;

        let (tx, _) = broadcast::channel(CAPACITY);
        let tx2 = tx.clone();

        tokio::task::spawn(async move {
            let mut current_block = self.from_block;
            loop {
                let res = self.run_inner(&tx2, &mut current_block).await;
                if let Err(err) = res {
                    log::error!("Gear block listener failed: {}", err);

                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Gear block listener reconnected");
                        }
                        Err(err) => {
                            log::error!("Gear block listener unable to reconnect: {err}");
                            return;
                        }
                    };
                }
            }
        });

        (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .expect("expected Vec of correct length")
    }

    async fn run_inner(
        &self,
        tx: &broadcast::Sender<GearBlock>,
        current_block: &mut u32,
    ) -> anyhow::Result<()> {
        self.metrics.latest_block.set(*current_block as i64);
        let gear_api = self.api_provider.client();
        loop {
            let finalized_head = gear_api.latest_finalized_block().await?;
            let finalized_head = gear_api.get_block_at(finalized_head).await?;

            let block_number_finalized = finalized_head.header().number();
            if block_number_finalized >= *current_block {
                for block in *current_block..block_number_finalized {
                    let block_hash = gear_api.block_number_to_hash(block).await?;
                    let block = gear_api.get_block_at(block_hash).await?;
                    let gear_block = GearBlock::from_subxt_block(block).await?;
                    tx.send(gear_block.clone())?;
                }

                let gear_block = GearBlock::from_subxt_block(finalized_head).await?;
                tx.send(gear_block.clone())?;

                *current_block = block_number_finalized + 1;
            } else {
                tokio::time::sleep(GEAR_BLOCK_TIME_APPROX).await;
            }
        }
    }
}
