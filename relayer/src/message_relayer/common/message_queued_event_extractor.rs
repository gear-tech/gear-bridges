use std::sync::mpsc::{channel, Receiver, Sender};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use prometheus::IntCounter;
use utils_prometheus::{impl_metered_service, MeteredService};

use super::{GearBlockNumber, MessageInBlock};

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
        total_messages_found: IntCounter,
    }
}

impl Metrics {
    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            total_messages_found: IntCounter::new(
                "message_queued_event_extractor_total_messages_found",
                "Total amount of messages discovered",
            )?,
        })
    }
}

impl MessageQueuedEventExtractor {
    pub fn new(gear_api: GearApi) -> Self {
        Self {
            gear_api,
            metrics: Metrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<GearBlockNumber>) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Message queued extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<MessageInBlock>,
        blocks: &Receiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlockNumber,
        sender: &Sender<MessageInBlock>,
    ) -> anyhow::Result<()> {
        let block_hash = self.gear_api.block_number_to_hash(block.0).await?;

        let messages = self.gear_api.message_queued_events(block_hash).await?;
        if !messages.is_empty() {
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
