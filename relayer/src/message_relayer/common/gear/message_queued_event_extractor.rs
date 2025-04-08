use gear_rpc_client::GearApi;
use prometheus::{Histogram, HistogramOpts, IntCounter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::common::{GearBlockNumber, MessageInBlock};

pub struct MessageQueuedEventExtractor {
    gear_api: GearApi,

    metrics: Metrics,
}

impl MeteredService for MessageQueuedEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        total_messages_found: IntCounter = IntCounter::new(
            "message_queued_event_extractor_total_messages_found",
            "Total amount of messages discovered",
        ),
        messages_per_block: Histogram = Histogram::with_opts(
            HistogramOpts::new(
                "message_queued_event_extractor_messages_per_block",
                "Number of MessageQueued events processed per Gear block",
            ).buckets(prometheus::exponential_buckets(1.0, 1.5, 50)?) // TODO(playX18): Experiment with this value more
        ),
        restarts: IntCounter = IntCounter::new(
            "message_queued_event_extractor_restarts",
            "Number of restarts of the message queued event extractor due to errors",
        ),
    }
}

impl MessageQueuedEventExtractor {
    pub fn new(gear_api: GearApi) -> Self {
        Self {
            gear_api,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        self,
        mut blocks: UnboundedReceiver<GearBlockNumber>,
    ) -> UnboundedReceiver<MessageInBlock> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    self.metrics.restarts.inc();
                    log::error!("Message queued extractor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &UnboundedSender<MessageInBlock>,
        blocks: &mut UnboundedReceiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            while let Ok(block) = blocks.try_recv() {
                self.process_block_events(block, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlockNumber,
        sender: &UnboundedSender<MessageInBlock>,
    ) -> anyhow::Result<()> {
        let block_hash = self.gear_api.block_number_to_hash(block.0).await?;

        let messages = self.gear_api.message_queued_events(block_hash).await?;

        if !messages.is_empty() {
            self.metrics
                .messages_per_block
                .observe(messages.len() as f64);
            log::info!(
                "Found {} queued messages in block #{}",
                messages.len(),
                block
            );
            self.metrics
                .total_messages_found
                .inc_by(messages.len() as u64);

            for message in messages {
                sender.send(MessageInBlock {
                    message,
                    block,
                    block_hash,
                })?;
            }
        }

        Ok(())
    }
}
