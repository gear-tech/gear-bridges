use bridging_payment_client::bridging_payment::events::BridgingPaymentEvents;

use primitive_types::H256;
use prometheus::IntCounter;
use sails_rs::events::EventIo;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{
    common::{gear::block_listener::GearBlock, PaidMessage},
    eth_to_gear::api_provider::ApiProviderConnection,
};

pub struct MessagePaidEventExtractor {
    bridging_payment_address: H256,

    api_provider: ApiProviderConnection,

    metrics: Metrics,
}

impl MeteredService for MessagePaidEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        total_messages_found: IntCounter = IntCounter::new(
            "message_paid_event_extractor_total_messages_found",
            "Total amount of paid messages discovered",
        ),
    }
}

impl MessagePaidEventExtractor {
    pub fn new(api_provider: ApiProviderConnection, bridging_payment_address: H256) -> Self {
        Self {
            bridging_payment_address,
            api_provider,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(mut self, mut blocks: Receiver<GearBlock>) -> UnboundedReceiver<PaidMessage> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    log::error!("Message paid event extractor failed: {}", err);

                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Message paid event extractor reconnected");
                        }

                        Err(err) => {
                            log::error!("Failed to reconnect: {}", err);
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
        sender: &UnboundedSender<PaidMessage>,
        blocks: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<()> {
        loop {
            match blocks.recv().await {
                Ok(block) => {
                    self.process_block_events(block, sender).await?;
                }
                Err(RecvError::Closed) => {
                    log::warn!("Message paid event extractor channel closed, exiting");
                    return Ok(());
                }

                Err(RecvError::Lagged(_)) => {
                    log::warn!("Message paid event extractor channel lagged behind, trying again");
                    continue;
                }
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlock,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let block_hash = block.hash();

        // As bridging-payment uses sails to send events, destnation will be zeroed.
        let destination = H256::zero();

        let messages = block.user_message_sent_events(self.bridging_payment_address, destination);

        log::info!(
            "Processing block #{} with hash {}",
            block.number(),
            block_hash
        );
        let mut total = 0;

        for message in messages {
            let user_reply = BridgingPaymentEvents::decode_event(message.payload)
                .map_err(|_| anyhow::anyhow!("Failed to decode bridging payment event"))?;

            let BridgingPaymentEvents::BridgingPaid { nonce } = user_reply;

            let mut nonce_le = [0; 32];
            nonce.to_little_endian(&mut nonce_le);

            sender.send(PaidMessage { nonce: nonce_le })?;
            total += 1;
        }
        log::info!("Found {} paid messages in block #{}", total, block.number());
        self.metrics.total_messages_found.inc_by(total as u64);

        Ok(())
    }
}
