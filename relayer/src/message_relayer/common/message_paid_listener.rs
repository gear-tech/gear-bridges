use std::sync::mpsc::{channel, Receiver, Sender};

use bridging_payment::services::BridgingPaymentEvents;
use futures::executor::block_on;
use gear_rpc_client::GearApi;
use parity_scale_codec::Decode;
use primitive_types::H256;
use prometheus::IntCounter;

use utils_prometheus::{impl_metered_service, MeteredService};

use super::block_listener::BlockNumber;

pub struct PaidMessage {
    pub nonce: [u8; 32],
}

pub struct MessagePaidListener {
    bridging_payment_address: H256,

    gear_api: GearApi,

    metrics: MessagePaidListenerMetrics,
}

impl MeteredService for MessagePaidListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct MessagePaidListenerMetrics {
        total_paid_messages_found: IntCounter,
    }
}

impl MessagePaidListenerMetrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            total_paid_messages_found: IntCounter::new(
                "message_relayer_message_paid_listener_total_paid_messages_found",
                "Total amount of paid messages found by event listener",
            )?,
        })
    }
}

impl MessagePaidListener {
    pub fn new(gear_api: GearApi, bridging_payment_address: H256) -> Self {
        Self {
            bridging_payment_address,
            gear_api,
            metrics: MessagePaidListenerMetrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<BlockNumber>) -> Receiver<PaidMessage> {
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
        sender: &Sender<PaidMessage>,
        blocks: &Receiver<BlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block.0, &sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: u32,
        sender: &Sender<PaidMessage>,
    ) -> anyhow::Result<()> {
        log::info!("Processing gear block #{}", block);
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        let messages = self
            .gear_api
            .user_message_sent_events(self.bridging_payment_address, block_hash)
            .await?;
        if !messages.is_empty() {
            log::info!("Found {} paid messages", messages.len());
            self.metrics
                .total_paid_messages_found
                .inc_by(messages.len() as u64);

            for message in messages {
                let user_reply = BridgingPaymentEvents::decode(&mut &message.payload[..])?;
                let BridgingPaymentEvents::TeleportVaraToEth { nonce, .. } = user_reply;

                let mut nonce_le = [0; 32];
                nonce.to_little_endian(&mut nonce_le);

                sender.send(PaidMessage { nonce: nonce_le })?;
            }
        }

        Ok(())
    }
}