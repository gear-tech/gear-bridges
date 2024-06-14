use std::{
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::Duration,
};

use bridging_payment::UserReply as BridgingPaymentUserReply;
use gear_rpc_client::GearApi;
use parity_scale_codec::Decode;
use primitive_types::H256;

use crate::metrics::{impl_metered_service, MeteredService};

use super::{BlockEvent, MessageInBlock};

const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

pub struct EventListener {
    gear_api: GearApi,
    from_block: u32,
    bridging_payment_address: Option<H256>,

    metrics: EventListenerMetrics,
}

impl MeteredService for EventListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct EventListenerMetrics {
        //
    }
}

impl EventListenerMetrics {
    fn new() -> Self {
        Self {}
    }
}

impl EventListener {
    pub fn new(gear_api: GearApi, from_block: u32, bridging_payment_address: Option<H256>) -> Self {
        Self {
            gear_api,
            from_block,
            bridging_payment_address,

            metrics: EventListenerMetrics::new(),
        }
    }

    pub fn run(self) -> Receiver<BlockEvent> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    log::error!("Event processor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(&self, sender: &Sender<BlockEvent>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        loop {
            let finalized_head = self.gear_api.latest_finalized_block().await?;
            let finalized_head = self.gear_api.block_hash_to_number(finalized_head).await?;

            if finalized_head >= current_block {
                for block in current_block..=finalized_head {
                    self.process_block_events(block, sender).await?;
                }

                current_block = finalized_head + 1;
            } else {
                thread::sleep(GEAR_BLOCK_TIME_APPROX);
            }
        }
    }

    async fn process_block_events(
        &self,
        block: u32,
        sender: &Sender<BlockEvent>,
    ) -> anyhow::Result<()> {
        log::info!("Processing gear block #{}", block);
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        let messages = self.gear_api.message_queued_events(block_hash).await?;
        if !messages.is_empty() {
            log::info!("Found {} sent messages", messages.len());

            for message in messages {
                sender.send(BlockEvent::MessageSent {
                    message: MessageInBlock {
                        message,
                        block,
                        block_hash,
                    },
                })?;
            }
        }

        if let Some(bridging_payment_address) = self.bridging_payment_address {
            let messages = self
                .gear_api
                .user_message_sent_events(bridging_payment_address, block_hash)
                .await?;
            if !messages.is_empty() {
                log::info!("Found {} paid messages", messages.len());

                for message in messages {
                    let user_reply = BridgingPaymentUserReply::decode(&mut &message.payload[..])?;
                    sender.send(BlockEvent::MessagePaid {
                        nonce: user_reply.nonce,
                    })?;
                }
            }
        }

        Ok(())
    }
}
