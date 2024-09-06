use std::sync::mpsc::{channel, Receiver, Sender};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use primitive_types::H256;
use prometheus::IntCounter;
use sails_rs::events::EventIo;
use utils_prometheus::{impl_metered_service, MeteredService};

use super::{GearBlockNumber, PaidMessage};

#[allow(dead_code)]
mod bridging_payment_client {
    use sails_rs::prelude::*;

    include!(concat!(env!("OUT_DIR"), "/bridging_payment_client.rs"));
}

use bridging_payment_client::bridging_payment::events::BridgingPaymentEvents;

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
        total_messages_found: IntCounter,
    }
}

impl Metrics {
    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            total_messages_found: IntCounter::new(
                "message_paid_event_extractor_total_messages_found",
                "Total amount of paid messages discovered",
            )?,
        })
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

    pub fn run(self, blocks: Receiver<GearBlockNumber>) -> Receiver<PaidMessage> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Message paid event extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<PaidMessage>,
        blocks: &Receiver<GearBlockNumber>,
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
        sender: &Sender<PaidMessage>,
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

            let BridgingPaymentEvents::TeleportVaraToEth { nonce, .. } = user_reply;

            let mut nonce_le = [0; 32];
            nonce.to_little_endian(&mut nonce_le);

            sender.send(PaidMessage { nonce: nonce_le })?;
        }

        Ok(())
    }
}
