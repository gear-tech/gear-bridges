use std::sync::mpsc::{channel, Receiver, Sender};

use block_listener::BlockNumber;
use futures::executor::block_on;
use gear_rpc_client::GearApi;
use prometheus::IntCounter;

use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::MessageInBlock;

pub mod block_listener;

pub struct EventListener {
    gear_api: GearApi,

    metrics: EventListenerMetrics,
}

impl MeteredService for EventListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct EventListenerMetrics {
        total_messages_found: IntCounter,
    }
}

impl EventListenerMetrics {
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

impl EventListener {
    pub fn new(gear_api: GearApi) -> Self {
        Self {
            gear_api,
            metrics: EventListenerMetrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<BlockNumber>) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                for block in blocks.try_iter() {
                    let res = block_on(self.process_block_events(block.0, &sender));
                    if let Err(err) = res {
                        log::error!("Event processor failed: {}", err);
                    }
                }
            }
        });

        receiver
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
