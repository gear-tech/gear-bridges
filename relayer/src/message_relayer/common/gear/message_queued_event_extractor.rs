use crate::message_relayer::{
    common::{gear::block_listener::GearBlock, AuthoritySetId, GearBlockNumber, MessageInBlock},
    eth_to_gear::api_provider::ApiProviderConnection,
};

use gsdk::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use prometheus::IntCounter;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::UnboundedSender,
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MessageQueuedEventExtractor {
    api_provider: ApiProviderConnection,
    sender: UnboundedSender<MessageInBlock>,

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

fn message_queued_events_of(
    block: &GearBlock,
) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
    block.events().iter().filter_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::MessageQueued { message, .. }) => {
            let mut nonce_le = [0; 32];
            primitive_types::U256(message.nonce.0).to_little_endian(&mut nonce_le);

            Some(gear_rpc_client::dto::Message {
                nonce_le,
                source: message.source.0,
                destination: message.destination.0,
                payload: message.payload.clone(),
            })
        }
        _ => None,
    })
}

impl MessageQueuedEventExtractor {
    pub fn new(api_provider: ApiProviderConnection, sender: UnboundedSender<MessageInBlock>,) -> Self {
        Self {
            api_provider,
            sender,
            metrics: Metrics::new(),
        }
    }

    pub fn spawn(
        mut self,
        mut blocks: Receiver<GearBlock>,
    ) {
        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&mut blocks).await;
                if let Err(err) = res {
                    log::error!("Message queued extractor failed: {err}");

                    match self.api_provider.reconnect().await {
                        Ok(_) => {
                            log::info!("Message queued extractor reconnected");
                        }
                        Err(err) => {
                            log::error!("Message queued extractor unable to reconnect: {err}");
                            return;
                        }
                    }
                } else {
                    log::debug!("MessageQueuedEventExtractor exiting...");
                    return;
                 }

            }
        });
    }

    async fn run_inner(
        &self,
        blocks: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        loop {
            match blocks.recv().await {
                Ok(block) => {
                    let block_hash = block.hash();
                    let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

                    self.process_block_events(block, authority_set_id)
                        .await?;
                }
                Err(RecvError::Closed) => {
                    log::warn!("Message queued extractor channel closed, exiting");
                    return Ok(());
                }
                Err(RecvError::Lagged(_)) => {
                    log::warn!("Message queued extractor channel lagged behind, trying again");
                    continue;
                }
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlock,
        authority_set_id: u64,
    ) -> anyhow::Result<()> {
        let messages = message_queued_events_of(&block);
        let block_hash = block.hash();
        let mut total = 0;
        for message in messages {
            total += 1;

            self.sender.send(MessageInBlock {
                message,
                block: GearBlockNumber(block.number()),
                block_hash,
                authority_set_id: AuthoritySetId(authority_set_id),
            })?;
        }

        if total > 0 {
            log::info!("Found {total} queued messages in block #{}", block.number());
        }

        self.metrics.total_messages_found.inc_by(total);

        Ok(())
    }
}
