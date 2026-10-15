use crate::{
    message_relayer::{
        common::{self, AuthoritySetId, GearBlock, GearBlockNumber, MessageInBlock},
        gear_to_eth::storage::{GearEventStream, Storage},
    },
    rpc,
};
use gear_common::api_provider::ApiProviderConnection;
use prometheus::IntCounter;
use std::{ops::RangeInclusive, sync::Arc, time::Duration};
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
            loop {
                match self.run_inner(&mut blocks).await {
                    Ok(()) => return,
                    Err(err) => {
                        log::error!("Message queued extractor failed: {err}");
                        if blocks.is_closed() || self.sender.is_closed() {
                            return;
                        }
                        loop {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            match self.api_provider.reconnect().await {
                                Ok(()) => break,
                                Err(err) => log::error!(
                                    "Message queued extractor unable to reconnect: {err}; retrying"
                                ),
                            }
                        }
                    }
                }
            }
        });
    }

    async fn run_inner(&mut self, blocks: &mut Receiver<GearBlock>) -> anyhow::Result<()> {
        self.replay_pending().await?;
        self.catch_up_to_finalized().await?;

        loop {
            match blocks.recv().await {
                Ok(block) => self.process_subscribed_block(block).await?,
                Err(RecvError::Closed) => {
                    log::warn!("Message queued extractor channel closed, exiting");
                    return Ok(());
                }
                Err(RecvError::Lagged(skipped)) => {
                    log::warn!("Message queued extractor lagged by {skipped} blocks; backfilling finalized range");
                    self.catch_up_to_finalized().await?;
                }
            }
        }
    }

    async fn replay_pending(&self) -> anyhow::Result<()> {
        let messages = self.storage.queued_observations().await?;
        let count = messages.len() as u64;
        for message in messages {
            self.sender.send(message)?;
        }
        self.metrics.total_messages_found.inc_by(count);
        Ok(())
    }

    async fn catch_up_to_finalized(&mut self) -> anyhow::Result<()> {
        let latest = latest_finalized_block_number(&mut self.api_provider).await?;
        self.catch_up_through(latest).await
    }

    async fn catch_up_through(&mut self, finalized_block: u32) -> anyhow::Result<()> {
        let start =
            self.storage.event_start_block().await?.ok_or_else(|| {
                anyhow::anyhow!("Gear event storage has not been bound to a chain")
            })?;
        let from = self
            .storage
            .replay_from_block(start, GearEventStream::Queued)
            .await?;
        let Some(range) = finalized_replay_range(from, finalized_block) else {
            return Ok(());
        };

        for block_number in range {
            let cursor = self.storage.event_cursor(GearEventStream::Queued).await?;
            if let Some(cursor) = cursor {
                if block_number < cursor.block {
                    continue;
                }
                if block_number == cursor.block {
                    let block =
                        fetch_finalized_gear_block(&mut self.api_provider, block_number).await?;
                    if block.hash() != cursor.hash {
                        anyhow::bail!("Finalized queued-event block #{block_number} changed hash");
                    }
                    continue;
                }
            }
            let block = fetch_finalized_gear_block(&mut self.api_provider, block_number).await?;
            self.process_block_events(block).await?;
        }
        Ok(())
    }

    async fn process_subscribed_block(&mut self, block: GearBlock) -> anyhow::Result<()> {
        let block_number = block.number();
        let block_hash = block.hash();
        let start =
            self.storage.event_start_block().await?.ok_or_else(|| {
                anyhow::anyhow!("Gear event storage has not been bound to a chain")
            })?;
        let cursor = self.storage.event_cursor(GearEventStream::Queued).await?;
        let next = match cursor {
            Some(cursor) if block_number <= cursor.block => {
                if block_number == cursor.block && block_hash != cursor.hash {
                    anyhow::bail!("Finalized queued-event block #{block_number} changed hash");
                }
                return Ok(());
            }
            Some(cursor) => cursor.block.checked_add(1),
            None => Some(start),
        };
        let Some(next) = next else {
            return Ok(());
        };
        if block_number < next {
            return Ok(());
        }
        if block_number > next {
            self.catch_up_through(block_number - 1).await?;
        }

        let next = self
            .storage
            .event_cursor(GearEventStream::Queued)
            .await?
            .map(|cursor| cursor.block.saturating_add(1))
            .unwrap_or(start);
        if block_number != next {
            anyhow::bail!(
                "Queued-event cursor is at #{}, cannot advance to #{block_number}",
                next.saturating_sub(1)
            );
        }
        self.process_block_events(block).await
    }

    async fn process_block_events(&mut self, block: GearBlock) -> anyhow::Result<()> {
        let block_number = block.number();
        let block_hash = block.hash();
        let messages = common::message_queued_events_of(&block).collect::<Vec<_>>();
        let authority_set_id = if messages.is_empty() {
            None
        } else {
            Some(
                rpc::retry_gear(
                    &mut self.api_provider,
                    "message queued authority set id",
                    move |gear_api| async move {
                        gear_api
                            .signed_by_authority_set_id(block_hash.0.into())
                            .await
                    },
                )
                .await?,
            )
        };
        let messages = messages
            .into_iter()
            .map(|message| MessageInBlock {
                message,
                block: GearBlockNumber(block_number),
                block_hash,
                authority_set_id: AuthoritySetId(
                    authority_set_id.expect("non-empty event list has an authority set"),
                ),
            })
            .collect::<Vec<_>>();

        self.storage
            .record_queued_block(block_number, block_hash, &messages)
            .await?;
        if !messages.is_empty() {
            self.storage
                .block_storage()
                .add_block(
                    GearBlockNumber(block_number),
                    block_hash,
                    messages.iter().map(|message| message.message.nonce_be),
                )
                .await;
        }

        let total = messages.len() as u64;
        for message in messages {
            self.sender.send(message)?;
        }
        if total > 0 {
            log::info!("Found {total} queued messages in block #{block_number}");
            self.metrics.total_messages_found.inc_by(total);
        }
        Ok(())
    }
}

pub async fn bind_event_storage(
    api_provider: &mut ApiProviderConnection,
    storage: &dyn Storage,
    configured_start: Option<u32>,
) -> anyhow::Result<u32> {
    let genesis_hash = rpc::retry_gear(api_provider, "Gear source genesis", |api| async move {
        api.block_number_to_hash(0).await
    })
    .await?;
    let saved_start = storage.event_start_block().await?;
    let start_block = match saved_start {
        Some(saved) => {
            if configured_start.is_some_and(|configured| configured != saved) {
                anyhow::bail!(
                    "Configured Gear start block {:?} differs from persisted start block {saved}",
                    configured_start
                );
            }
            saved
        }
        None => match configured_start {
            Some(configured) => configured,
            None => latest_finalized_block_number(api_provider)
                .await?
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Latest finalized Gear block number overflowed"))?,
        },
    };
    storage.bind_event_chain(genesis_hash, start_block).await?;
    Ok(start_block)
}

pub(crate) async fn latest_finalized_block_number(
    api_provider: &mut ApiProviderConnection,
) -> anyhow::Result<u32> {
    rpc::retry_gear(
        api_provider,
        "Gear latest finalized block",
        |api| async move {
            let hash = api.latest_finalized_block().await?;
            api.block_hash_to_number(hash).await
        },
    )
    .await
}

pub(crate) async fn fetch_finalized_gear_block(
    api_provider: &mut ApiProviderConnection,
    block_number: u32,
) -> anyhow::Result<GearBlock> {
    rpc::retry_gear(
        api_provider,
        "Gear finalized block backfill",
        move |api| async move {
            let block_hash = api.block_number_to_hash(block_number).await?;
            let block = api.api.blocks().at(block_hash).await?;
            GearBlock::from_finalized_block(&api, block).await
        },
    )
    .await
}

pub(crate) fn finalized_replay_range(from: u32, through: u32) -> Option<RangeInclusive<u32>> {
    (from <= through).then_some(from..=through)
}

#[cfg(test)]
mod tests {
    use super::finalized_replay_range;

    #[test]
    fn replay_range_covers_lag_and_resumes_after_saved_cursor() {
        assert_eq!(finalized_replay_range(11, 13), Some(11..=13));
        assert_eq!(finalized_replay_range(14, 13), None);
    }
}
