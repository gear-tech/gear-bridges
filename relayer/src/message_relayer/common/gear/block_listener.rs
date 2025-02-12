use std::{
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{self, common::{GSdkArgs, GearBlockNumber, gear::checkpoints_extractor::Request}};

const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

pub struct BlockListener {
    sender: tokio::sync::mpsc::Sender<Request>,
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
    pub fn new(from_block: u32, sender: tokio::sync::mpsc::Sender<Request>,) -> Self {
        Self {
            sender,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub fn run<const RECEIVER_COUNT: usize>(self) -> [Receiver<GearBlockNumber>; RECEIVER_COUNT] {
        let (senders, receivers): (Vec<_>, Vec<_>) = (0..RECEIVER_COUNT).map(|_| channel()).unzip();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&senders));
            if let Err(err) = res {
                log::error!("Gear block listener failed: {}", err);
            }
        });

        receivers
            .try_into()
            .expect("Expected Vec of correct length")
    }

    async fn run_inner(&self, senders: &[Sender<GearBlockNumber>]) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        self.metrics.latest_block.set(current_block as i64);

        loop {
            let from_gear_block = {
                let (sender, mut reciever) = tokio::sync::oneshot::channel();
                let request = message_relayer::common::gear::checkpoints_extractor::Request::LatestFinalizedBlock { sender };

                // todo: exit
                self.sender.send(request).await?;

                reciever.await??
            };
            let finalized_head = {
                let (sender, mut reciever) = tokio::sync::oneshot::channel();
                let request = message_relayer::common::gear::checkpoints_extractor::Request::BlockHashToNumber { hash: from_gear_block, sender };

                // todo: exit
                self.sender.send(request).await?;

                reciever.await??
            };

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
