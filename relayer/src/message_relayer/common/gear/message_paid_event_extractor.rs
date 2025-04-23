use gear_rpc_client::GearApi;
use primitive_types::H256;
use prometheus::IntCounter;
use sails_rs::events::EventIo;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use bridging_payment_client::bridging_payment::events::BridgingPaymentEvents;

use crate::message_relayer::common::{GearBlockNumber, PaidMessage};

pub struct MessagePaidEventExtractor {
    bridging_payment_address: H256,

    gear_api: GearApi,

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
    pub fn new(gear_api: GearApi, bridging_payment_address: H256) -> Self {
        Self {
            bridging_payment_address,
            gear_api,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        self,
        mut blocks: UnboundedReceiver<GearBlockNumber>,
    ) -> UnboundedReceiver<PaidMessage> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    log::error!("Message paid event extractor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &UnboundedSender<PaidMessage>,
        blocks: &mut UnboundedReceiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            while let Some(block) = blocks.recv().await {
                self.process_block_events(block.0, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: u32,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        // As bridging-payment uses sails to send events, destnation will be zeroed.
        let destination = H256::zero();

        let messages = self
            .gear_api
            .user_message_sent_events(self.bridging_payment_address, destination, block_hash)
            .await?;

        if messages.is_empty() {
            return Ok(());
        }

        log::info!("Found {} paid messages at block #{}", messages.len(), block);

        self.metrics
            .total_messages_found
            .inc_by(messages.len() as u64);

        for message in messages {
            let user_reply = BridgingPaymentEvents::decode_event(message.payload)
                .map_err(|_| anyhow::anyhow!("Failed to decode bridging payment event"))?;

            let BridgingPaymentEvents::BridgingPaid { nonce } = user_reply;

            let mut nonce_le = [0; 32];
            nonce.to_little_endian(&mut nonce_le);

            sender.send(PaidMessage { nonce: nonce_le })?;
        }

        Ok(())
    }
}
