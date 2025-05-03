use prometheus::IntCounter;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{
    common::{GearBlockNumber, MessageInBlock},
    eth_to_gear::api_provider::ApiProviderConnection,
};

pub struct MessageQueuedEventExtractor {
    api_provider: ApiProviderConnection,

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
    }
}

impl MessageQueuedEventExtractor {
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self {
            api_provider,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: UnboundedReceiver<GearBlockNumber>,
    ) -> UnboundedReceiver<MessageInBlock> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    log::error!("Message queued extractor failed: {}", err);

                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Gear block listener reconnected");
                        }
                        Err(err) => {
                            log::error!("Gear block listener unable to reconnect: {err}");
                            return;
                        }
                    }
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
            while let Some(block) = blocks.recv().await {
                self.process_block_events(block, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlockNumber,
        sender: &UnboundedSender<MessageInBlock>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        let block_hash = gear_api.block_number_to_hash(block.0).await?;

        let messages = gear_api.message_queued_events(block_hash).await?;
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
