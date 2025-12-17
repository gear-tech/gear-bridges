use crate::message_relayer::{
    common::{
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
};
use futures::StreamExt;
use primitive_types::H256;
use prometheus::IntGauge;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct BlockListener {
    api_provider: ApiProviderConnection,

    block_storage: Arc<dyn UnprocessedBlocksStorage>,

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
        Self {
            api_provider,
            block_storage,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        // Capacity for the channel. At the moment merkle-root relayer might lag behind
        // during proof generation or era sync, so we need to have enough capacity
        // to not drop any blocks. 14400 is how many blocks are produced in 1 era.
        const CAPACITY: usize = 14_400;
        let (tx, _) = broadcast::channel(CAPACITY);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            let api = self.api_provider.client();
            let UnprocessedBlocks {
                last_block,
                first_block,
                blocks,
            } = self.block_storage.unprocessed_blocks().await;

            // Fixed boundary for the initial backlog fetch.
            // We fetch blocks up to this number, then only process subscription blocks above it.
            let from_block = first_block.or(last_block);
            let mut backlog_end: Option<u32> = None;
            let mut latest_finalized_hash: Option<H256> = None;

            if from_block.is_some() {
                match api.latest_finalized_block().await {
                    Ok(block_hash) => match api.block_hash_to_number(block_hash).await {
                        Ok(number) => {
                            backlog_end = Some(number);
                            latest_finalized_hash = Some(block_hash);
                            self.metrics.latest_block.set(number as i64);
                        }
                        Err(err) => {
                            log::error!(
                                "Failed to convert latest finalized block hash to number: {err}"
                            );
                            return;
                        }
                    },
                    Err(err) => {
                        log::error!("Failed to get latest finalized block: {err}");
                        return;
                    }
                }
            }

            // Compute + fetch the backlog in a separate task.
            // This keeps the subscription loop responsive.
            let backlog_handle = self.spawn_backlog_task(
                tx2.clone(),
                blocks,
                first_block,
                from_block,
                backlog_end,
                latest_finalized_hash,
            );

            let mut backlog_handle = backlog_handle;
            let mut last_finalized_block_number = backlog_end;

            loop {
                let res = self
                    .run_subscription_loop(
                        &tx2,
                        &mut last_finalized_block_number,
                        &mut backlog_handle,
                    )
                    .await;
                match res {
                    Ok(false) => {
                        log::info!("Gear block listener stopped due to no active receivers");
                        if let Some(handle) = backlog_handle.take() {
                            handle.abort();
                        }
                        return;
                    }

                    Ok(true) => {
                        log::info!("Gear block listener: subscription expired, restarting");
                        continue;
                    }

                    Err(err) => {
                        log::error!("Gear block listener failed: {err}");

                        match self.api_provider.reconnect().await {
                            Ok(()) => {
                                log::info!("Gear block listener reconnected");
                            }
                            Err(err) => {
                                log::error!("Gear block listener unable to reconnect: {err}");
                                return;
                            }
                        };
                    }
                }
            }
        });

        (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .expect("expected Vec of correct length")
    }

    fn spawn_backlog_task(
        &self,
        tx: broadcast::Sender<GearBlock>,
        mut stored_blocks: Vec<(H256, u32)>,
        first_block: Option<(H256, u32)>,
        from_block: Option<(H256, u32)>,
        backlog_end: Option<u32>,
        latest_finalized_hash: Option<H256>,
    ) -> Option<tokio::task::JoinHandle<anyhow::Result<()>>> {
        if stored_blocks.is_empty() && backlog_end.is_none() {
            return None;
        }

        // Ensure deterministic order and avoid duplicates from storage.
        stored_blocks.sort_by_key(|(_, n)| *n);
        stored_blocks.dedup_by_key(|(_, n)| *n);

        let mut already_known_numbers: HashSet<u32> =
            stored_blocks.iter().map(|(_, n)| *n).collect();

        let api_provider = self.api_provider.clone();
        let block_storage = self.block_storage.clone();

        Some(tokio::task::spawn(async move {
            let gear_api = api_provider.client();

            // 1) Send the exact set of blocks the storage says are unprocessed.
            for (block_hash, block_number) in stored_blocks {
                log::trace!(
                    "Fetching stored unprocessed block #{block_number} (hash: {block_hash})"
                );
                let block = gear_api.api.blocks().at(block_hash).await?;
                let gear_block = GearBlock::from_subxt_block(&gear_api, block).await?;
                if tx.send(gear_block).is_err() {
                    // No receivers; stop quietly.
                    return Ok(());
                }
            }

            // 2) If requested by the storage (first/last block present), extend backlog up to
            //    the fixed boundary `backlog_end`.
            let Some(backlog_end) = backlog_end else {
                return Ok(());
            };
            let Some(from_block) = from_block else {
                return Ok(());
            };

            let start = if first_block.is_some() {
                from_block.1
            } else {
                from_block.1.saturating_add(1)
            };

            let latest_finalized_hash = latest_finalized_hash
                .ok_or_else(|| anyhow::anyhow!("latest finalized hash is missing"))?;

            for block_number in start..=backlog_end {
                if already_known_numbers.contains(&block_number) {
                    continue;
                }          

                let block_hash = if block_number == backlog_end {
                    latest_finalized_hash
                } else {
                    match gear_api.block_number_to_hash(block_number).await {
                        Ok(hash) => hash,
                        Err(err) => {
                            log::error!(
                                "Failed to get block hash for number {block_number}: {err}"
                            );
                            continue;
                        }
                    }
                };

                log::trace!("Fetching backlog block #{block_number} (hash: {block_hash})");
                let block = gear_api.api.blocks().at(block_hash).await?;
                let gear_block = GearBlock::from_subxt_block(&gear_api, block).await?;

                // Only blocks fetched from RPC need to be added to storage.
                block_storage.add_block(&gear_api, &gear_block).await?;

                if tx.send(gear_block).is_err() {
                    // No receivers; stop quietly.
                    return Ok(());
                }
            }

            Ok(())
        }))
    }

    async fn drain_backlog_if_finished(
        backlog_handle: &mut Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    ) -> anyhow::Result<()> {
        let Some(handle) = backlog_handle.as_ref() else {
            return Ok(());
        };

        if !handle.is_finished() {
            return Ok(());
        }

        // Take and await the handle only if it's finished, so we never block the subscription loop.
        let handle = backlog_handle.take().expect("handle exists");
        handle
            .await
            .map_err(|err| anyhow::anyhow!("Backlog task panicked or was cancelled: {err}"))?
    }

    async fn run_subscription_loop(
        &self,
        tx: &broadcast::Sender<GearBlock>,

        last_finalized_block_number: &mut Option<u32>,
        backlog_handle: &mut Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    ) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();
        let mut subscription = gear_api.subscribe_grandpa_justifications().await?;

        while let Some(justification) = subscription.next().await {
            Self::drain_backlog_if_finished(backlog_handle).await?;

            let justification = justification?;

            let should_continue = self
                .process_justification(&gear_api, tx, last_finalized_block_number, justification)
                .await?;
            if !should_continue {
                return Ok(false);
            }
        }

        // Subscription expired.
        Ok(true)
    }

    async fn process_justification(
        &self,
        gear_api: &gear_rpc_client::GearApi,
        tx: &broadcast::Sender<GearBlock>,
        last_finalized_block_number: &mut Option<u32>,
        justification: sp_consensus_grandpa::GrandpaJustification<gear_rpc_client::GearHeader>,
    ) -> anyhow::Result<bool> {
        let block_hash = justification.commit.target_hash;
        let block_number = justification.commit.target_number;

        // Check if there are missing blocks and fetch them
        if let Some(last_finalized) = *last_finalized_block_number {
            if last_finalized + 1 != block_number {
                log::info!(
                    "Detected gap: last finalized block was #{last_finalized}, current block is #{block_number}"
                );

                for missing_block in (last_finalized + 1)..block_number {
                    log::trace!("Fetching missing block #{missing_block}");

                    let missing_block_hash =
                        match gear_api.block_number_to_hash(missing_block).await {
                            Ok(hash) => hash,
                            Err(err) => {
                                log::error!(
                                "Failed to get block hash for missing block #{missing_block}: {err}"
                            );
                                continue;
                            }
                        };

                    let missing_block_data = gear_api.api.blocks().at(missing_block_hash).await?;
                    let gear_block =
                        GearBlock::from_subxt_block(gear_api, missing_block_data).await?;
                    self.block_storage.add_block(gear_api, &gear_block).await?;
                    if tx.send(gear_block).is_err() {
                        log::error!("No active receivers for Gear block listener, stopping");
                        return Ok(false);
                    }
                }
            }
        }

        let block = gear_api.api.blocks().at(block_hash).await?;
        let gear_block = GearBlock::from_subxt_block(gear_api, block).await?;
        self.block_storage.add_block(gear_api, &gear_block).await?;
        if tx.send(gear_block).is_err() {
            log::error!("No active receivers for Gear block listener, stopping");
            return Ok(false);
        }

        *last_finalized_block_number = Some(block_number);
        self.metrics.latest_block.set(block_number as i64);

        Ok(true)
    }
}
