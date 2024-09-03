use std::sync::mpsc::{channel, Receiver, Sender};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use prometheus::IntCounter;

use utils_prometheus::{impl_metered_service, MeteredService};

use super::{gear_block_listener::BlockNumber, MessageInBlock};

pub struct MessageQueuedEventExtractor {
    gear_api: GearApi,

    metrics: MessageQueuedListenerMetrics,
}

impl MeteredService for MessageQueuedEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct MessageQueuedListenerMetrics {
        total_messages_found: IntCounter,
    }
}

impl MessageQueuedListenerMetrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            total_messages_found: IntCounter::new(
                "message_relayer_event_listener_total_messages_found",
                "Total amount of messages found by event listener, including not paid",
            )?,
        })
    }
}

impl MessageQueuedEventExtractor {
    pub fn new(gear_api: GearApi) -> Self {
        Self {
            gear_api,
            metrics: MessageQueuedListenerMetrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<BlockNumber>) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = block_on(self.run_inner(&sender, &blocks));
                if let Err(err) = res {
                    log::error!("Event processor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<MessageInBlock>,
        blocks: &Receiver<BlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block.0, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: u32,
        sender: &Sender<MessageInBlock>,
    ) -> anyhow::Result<()> {
        log::info!("Processing gear block #{}", block);
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        let messages = self.gear_api.message_queued_events(block_hash).await?;
        if !messages.is_empty() {
            log::info!("Found {} sent messages", messages.len());
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
