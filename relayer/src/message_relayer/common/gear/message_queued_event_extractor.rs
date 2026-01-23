use std::sync::Arc;

use crate::message_relayer::{
    common::{self, AuthoritySetId, GearBlock, GearBlockNumber, MessageInBlock},
    eth_to_gear::api_provider::ApiProviderConnection,
    gear_to_eth::storage::Storage,
};
use prometheus::IntCounter;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::UnboundedSender,
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MessageQueuedEventExtractor {
    api_provider: ApiProviderConnection,
    sender: UnboundedSender<MessageInBlock>,
    storage: Arc<dyn Storage>,

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
    pub fn new(
        api_provider: ApiProviderConnection,
        sender: UnboundedSender<MessageInBlock>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        Self {
            api_provider,
            sender,
            storage,
            metrics: Metrics::new(),
        }
    }

    pub fn spawn(mut self, mut blocks: Receiver<GearBlock>) {
        tokio::task::spawn(async move {
            let mut pending_block: Option<GearBlock> = None;

            loop {
                // If we have a pending block, we'll try to process it first inside run_inner
                let res = self.run_inner(&mut blocks, &mut pending_block).await;
                if let Err(err) = res {
                    log::error!("Message queued extractor failed: {err}");

                    if blocks.is_closed() {
                        return;
                    }

                    // Loop until success
                    loop {
                         match self.api_provider.reconnect().await {
                            Ok(_) => {
                                log::info!("Message queued extractor reconnected");
                                break;
                            }
                            Err(err) => {
                                log::error!("Message queued extractor unable to reconnect: {err}. Retrying in 5s...");
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
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
        pending_block: &mut Option<GearBlock>
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        loop {
            // Use pending block if available, otherwise receive new one
            let block = if let Some(block) = pending_block.take() {
                block
            } else {
                match blocks.recv().await {
                    Ok(block) => block,
                    Err(RecvError::Closed) => {
                        log::warn!("Message queued extractor channel closed, exiting");
                        return Ok(());
                    }
                    Err(RecvError::Lagged(_)) => {
                        log::warn!("Message queued extractor channel lagged behind, trying again");
                        continue;
                    }
                }
            };
            
            // At this point we have a block. If we fail, we MUST put it back into pending_block.
            let block_hash = block.hash();
            match gear_api
                .signed_by_authority_set_id(block_hash.0.into())
                .await
            {
                Ok(authority_set_id) => {
                     match self.process_block_events(block.clone(), authority_set_id).await {
                        Ok(_) => {},
                        Err(e) => {
                             *pending_block = Some(block);
                             return Err(e);
                        }
                     }
                }
                Err(e) => {
                    *pending_block = Some(block);
                    return Err(e.into());
                }
            }
        }
    }

    async fn process_block_events(
        &self,
        block: GearBlock,
        authority_set_id: u64,
    ) -> anyhow::Result<()> {
        let messages = common::message_queued_events_of(&block).collect::<Vec<_>>();
        let block_hash = block.hash();
        let mut total = 0;

        self.storage
            .block_storage()
            .add_block(
                GearBlockNumber(block.number()),
                block_hash,
                messages.iter().map(|message| message.nonce_be),
            )
            .await;

        for message in messages {
            if !self
                .storage
                .block_storage()
                .is_message_pending(GearBlockNumber(block.number()), message.nonce_be)
                .await
            {
                // Message already dequeued (in-flight or completed) according to persisted storage.
                continue;
            }

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
