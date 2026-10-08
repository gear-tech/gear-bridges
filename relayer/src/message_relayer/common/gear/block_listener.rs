use crate::{
    message_relayer::common::{
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock,
    },
    rpc,
};
use futures::StreamExt;
use gear_common::api_provider::ApiProviderConnection;
use prometheus::IntGauge;
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct BlockListener {
    api_provider: ApiProviderConnection,

    block_storage: Arc<dyn UnprocessedBlocksStorage>,
    relayer_id: String,
    start_block: Option<u32>,

    metrics: Metrics,
}

impl MeteredService for BlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_block: IntGauge = IntGauge::new(
            "gear_block_listener_latest_block",
            "Latest gear block discovered by gear block listener",
        )
    }
}

impl BlockListener {
    pub fn new(
        api_provider: ApiProviderConnection,
        block_storage: Arc<dyn UnprocessedBlocksStorage>,
    ) -> Self {
        Self::new_for_relayer(api_provider, block_storage, "unlabeled".to_string())
    }

    pub fn new_for_relayer(
        api_provider: ApiProviderConnection,
        block_storage: Arc<dyn UnprocessedBlocksStorage>,
        relayer_id: String,
    ) -> Self {
        Self {
            api_provider,
            block_storage,
            relayer_id,
            start_block: None,

            metrics: Metrics::new(),
        }
    }

    pub fn start_from(mut self, block: Option<u32>) -> Self {
        self.start_block = block;
        self
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        // Leave room for one full Gear era while slow consumers finish their work.
        const CAPACITY: usize = 14_400;
        let (tx, _) = broadcast::channel(CAPACITY);
        let receivers = (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .expect("expected Vec of correct length");
        let sender = tx.clone();

        tokio::task::spawn(async move {
            let relayer_id = self.relayer_id.clone();
            let UnprocessedBlocks {
                last_block,
                first_block,
                blocks: _,
            } = self.block_storage.unprocessed_blocks().await;
            let mut last_finalized_block_number = None;
            let start_block = first_block
                .or(last_block)
                .map(|block| block.1)
                .or(self.start_block);
            if let Some(start_block) = start_block {
                log::info!(
                    "Gear block listener for relayer {relayer_id}: catching up from #{start_block}",
                );
                if !self
                    .replay_until_caught_up(
                        &sender,
                        start_block,
                        &mut last_finalized_block_number,
                        "startup catch-up",
                    )
                    .await
                {
                    return;
                }
            }

            loop {
                let result = self
                    .run_inner(&sender, &mut last_finalized_block_number)
                    .await;
                match result {
                    Ok(false) => {
                        log::info!(
                            "Gear block listener for relayer {relayer_id} stopped due to no active receivers"
                        );
                        return;
                    }
                    Ok(true) => {
                        log::info!(
                            "Gear block listener for relayer {relayer_id}: subscription expired, restarting"
                        );
                    }
                    Err(err) => {
                        log::error!(
                            r#"Gear block listener for relayer {relayer_id} failed: "{err:?}""#
                        );
                        loop {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            match self.api_provider.reconnect().await {
                                Ok(()) => {
                                    log::debug!(
                                        "Gear block listener for relayer {relayer_id}: API provider reconnected"
                                    );
                                    break;
                                }
                                Err(err) => log::error!(
                                    r#"Gear block listener for relayer {relayer_id}: API provider unable to reconnect: "{err}""#
                                ),
                            }
                        }
                    }
                }

                if let Some(from_block) = last_finalized_block_number
                    .map(|block| block.saturating_add(1))
                    .or(self.start_block)
                {
                    if !self
                        .replay_until_caught_up(
                            &sender,
                            from_block,
                            &mut last_finalized_block_number,
                            "reconnect catch-up",
                        )
                        .await
                    {
                        return;
                    }
                }
            }
        });

        receivers
    }

    async fn run_inner(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        last_finalized_block_number: &mut Option<u32>,
    ) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();

        let mut subscription = gear_api.subscribe_grandpa_justifications().await?;
        while let Some(justification) = subscription.next().await {
            let justification = justification?;

            let block_hash = justification.commit.target_hash;
            let block_number = justification.commit.target_number;

            if let Some(range) = missing_finalized_range(*last_finalized_block_number, block_number)
            {
                let from_block = *range.start();
                let to_block = *range.end();
                log::info!(
                    "Gear block listener for relayer {}: replaying missing finalized blocks #{from_block}..=#{to_block} before #{block_number}",
                    self.relayer_id
                );
                if !self
                    .replay_range(
                        tx,
                        from_block,
                        to_block,
                        last_finalized_block_number,
                        "live gap replay",
                    )
                    .await?
                {
                    return Ok(false);
                }
            }
            if last_finalized_block_number.is_some_and(|last| block_number <= last) {
                continue;
            }

            // Process the current block
            if !self
                .fetch_store_send(tx, block_number, Some(block_hash.0.into()))
                .await?
            {
                return Ok(false);
            }

            // Update the last finalized block number
            *last_finalized_block_number = Some(block_number);
            self.metrics.latest_block.set(block_number as i64);
        }

        Ok(true)
    }

    async fn replay_to_latest(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        from_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) -> anyhow::Result<bool> {
        let latest = rpc::retry_gear(
            &mut self.api_provider,
            "gear latest finalized block",
            |api| async move {
                let hash = api.latest_finalized_block().await?;
                api.block_hash_to_number(hash).await
            },
        )
        .await?;
        if from_block > latest {
            return Ok(true);
        }

        self.replay_range(tx, from_block, latest, last_finalized_block_number, reason)
            .await
    }

    async fn replay_until_caught_up(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        start_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) -> bool {
        loop {
            let from_block = last_finalized_block_number
                .map(|block| block.saturating_add(1))
                .unwrap_or(start_block);
            match self
                .replay_to_latest(tx, from_block, last_finalized_block_number, reason)
                .await
            {
                Ok(complete) => return complete,
                Err(err) => {
                    log::error!(
                        "Gear block listener for relayer {} {reason} failed: {err}",
                        self.relayer_id
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if let Err(err) = self.api_provider.reconnect().await {
                        log::error!(
                            "Gear block listener for relayer {} reconnect failed: {err}",
                            self.relayer_id
                        );
                    }
                }
            }
        }
    }

    async fn replay_range(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        from_block: u32,
        to_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) -> anyhow::Result<bool> {
        if from_block > to_block {
            return Ok(true);
        }
        log::info!(
            "Gear block listener for relayer {} {reason}: replaying blocks #{from_block}..=#{to_block}",
            self.relayer_id
        );
        for block_number in from_block..=to_block {
            if !self.fetch_store_send(tx, block_number, None).await? {
                return Ok(false);
            }
            *last_finalized_block_number = Some(block_number);
            self.metrics.latest_block.set(block_number as i64);
        }
        Ok(true)
    }

    async fn fetch_store_send(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        block_number: u32,
        known_hash: Option<primitive_types::H256>,
    ) -> anyhow::Result<bool> {
        let storage = self.block_storage.clone();
        let gear_block = rpc::retry_gear(
            &mut self.api_provider,
            "gear finalized block replay",
            move |api| {
                let storage = storage.clone();
                async move {
                    let block_hash = match known_hash {
                        Some(hash) => hash,
                        None => api.block_number_to_hash(block_number).await?,
                    };
                    let block = api.api.blocks().at(block_hash).await?;
                    let gear_block = if storage.requires_finality_proof() {
                        GearBlock::from_subxt_block(&api, block).await?
                    } else {
                        GearBlock::from_finalized_block(&api, block).await?
                    };
                    storage.add_block(&api, &gear_block).await?;
                    Ok(gear_block)
                }
            },
        )
        .await?;
        if tx.send(gear_block).is_err() {
            log::error!(
                "Gear block listener for relayer {}: no active receivers, stopping",
                self.relayer_id
            );
            return Ok(false);
        }
        Ok(true)
    }
}

fn missing_finalized_range(
    last_finalized: Option<u32>,
    current: u32,
) -> Option<std::ops::RangeInclusive<u32>> {
    let first_missing = last_finalized?.checked_add(1)?;
    (current > first_missing).then(|| first_missing..=current - 1)
}

#[cfg(test)]
mod tests {
    use super::missing_finalized_range;

    #[test]
    fn replays_missing_finalized_blocks_before_the_live_block() {
        assert_eq!(missing_finalized_range(Some(10), 13), Some(11..=12));
        assert_eq!(missing_finalized_range(Some(10), 11), None);
        assert_eq!(missing_finalized_range(Some(10), 10), None);
        assert_eq!(missing_finalized_range(None, 13), None);
    }
}
