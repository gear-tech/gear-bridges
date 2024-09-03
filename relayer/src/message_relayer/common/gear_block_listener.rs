use std::{
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use gear_rpc_client::GearApi;
use prometheus::IntGauge;

use utils_prometheus::{impl_metered_service, MeteredService};

use super::GearBlockNumber;

const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

pub struct GearBlockListener {
    gear_api: GearApi,
    from_block: u32,

    metrics: BlockListenerMetrics,
}

impl MeteredService for GearBlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct BlockListenerMetrics {
        processed_block: IntGauge,
    }
}

impl BlockListenerMetrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            processed_block: IntGauge::new(
                "message_relayer_event_listener_processed_block",
                "Gear block processed by event listener",
            )?,
        })
    }
}

impl GearBlockListener {
    pub fn new(gear_api: GearApi, from_block: u32) -> Self {
        Self {
            gear_api,
            from_block,

            metrics: BlockListenerMetrics::new(),
        }
    }

    pub fn run<const RECEIVER_COUNT: usize>(self) -> [Receiver<GearBlockNumber>; RECEIVER_COUNT] {
        let (senders, receivers): (Vec<_>, Vec<_>) = (0..RECEIVER_COUNT).map(|_| channel()).unzip();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&senders).await;
                if let Err(err) = res {
                    log::error!("Block listener failed: {}", err);
                }
            }
        });

        receivers
            .try_into()
            .expect("Expected Vec of correct length")
    }

    async fn run_inner(&self, senders: &[Sender<GearBlockNumber>]) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.processed_block.set(current_block as i64);

        loop {
            let finalized_head = self.gear_api.latest_finalized_block().await?;
            let finalized_head = self.gear_api.block_hash_to_number(finalized_head).await?;

            if finalized_head >= current_block {
                for block in current_block..=finalized_head {
                    for sender in senders {
                        sender.send(GearBlockNumber(block))?;
                    }

                    self.metrics.processed_block.inc();
                }

                current_block = finalized_head + 1;
            } else {
                tokio::time::sleep(GEAR_BLOCK_TIME_APPROX).await;
            }
        }
    }
}
