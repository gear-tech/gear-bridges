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
use std::sync::Arc;
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
                blocks: _,
            } = self.block_storage.unprocessed_blocks().await;

            let mut unprocessed = Vec::new();

            if let Some(from_block) = last_block.or(first_block) {
                let latest_finalized_block = match api.latest_finalized_block().await {
                    Ok(block) => block,
                    Err(err) => {
                        log::error!("Failed to get latest finalized block: {err}");
                        return;
                    }
                };
                let Ok(latest_finalized_block_number) =
                    api.block_hash_to_number(latest_finalized_block).await
                else {
                    log::error!("Failed to convert latest finalized block hash to number");
                    return;
                };

                for block in from_block.1..=latest_finalized_block_number {
                    let hash = if block == latest_finalized_block_number {
                        latest_finalized_block
                    } else {
                        match api.block_number_to_hash(block).await {
                            Ok(hash) => hash,
                            Err(err) => {
                                log::error!("Failed to get block hash for number {block}: {err}");
                                continue;
                            }
                        }
                    };
                    unprocessed.push((hash, block));
                }
            }

            loop {
                let res = self.run_inner(&tx2, &mut unprocessed).await;
                match res {
                    Ok(false) => {
                        log::info!("Gear block listener stopped due to no active receivers");
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

    async fn run_inner(
        &self,
        tx: &broadcast::Sender<GearBlock>,
        unprocessed: &mut Vec<(H256, u32)>,
    ) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();

        for (block_hash, block_number) in unprocessed.drain(..) {
            log::trace!("Fetching unprocessed block #{block_number} (hash: {block_hash})");
            let block = gear_api.api.blocks().at(block_hash).await?;

            let gear_block = GearBlock::from_subxt_block(&gear_api, block).await?;

            match tx.send(gear_block) {
                Ok(_) => (),
                Err(broadcast::error::SendError(_)) => {
                    log::error!("No active receivers for Gear block listener, stopping");
                    return Ok(false);
                }
            }
        }

        let mut last_finalized_block_number = None;
        let mut subscription = gear_api.subscribe_grandpa_justifications().await?;

        while let Some(justification) = subscription.next().await {
            let justification = justification?;

            let block_hash = justification.commit.target_hash;
            let block_number = justification.commit.target_number;

            // Check if there are missing blocks and fetch them
            if let Some(last_finalized) = last_finalized_block_number {
                if last_finalized + 1 != block_number {
                    log::info!("Detected gap: last finalized block was #{last_finalized}, current block is #{block_number}");

                    // Fetch missing blocks
                    for missing_block in (last_finalized + 1)..block_number {
                        log::trace!("Fetching missing block #{missing_block}");

                        let missing_block_hash = match gear_api
                            .block_number_to_hash(missing_block)
                            .await
                        {
                            Ok(hash) => hash,
                            Err(err) => {
                                log::error!("Failed to get block hash for missing block #{missing_block}: {err}");
                                continue;
                            }
                        };

                        let missing_block_data =
                            gear_api.api.blocks().at(missing_block_hash).await?;
                        let gear_block =
                            GearBlock::from_subxt_block(&gear_api, missing_block_data).await?;
                        self.block_storage.add_block(&gear_api, &gear_block).await?;
                        match tx.send(gear_block) {
                            Ok(_) => {}
                            Err(broadcast::error::SendError(_)) => {
                                log::error!(
                                    "No active receivers for Gear block listener, stopping"
                                );
                                return Ok(false);
                            }
                        }
                    }
                }
            }

            // Process the current block
            let block_hash: primitive_types::H256 = block_hash.0.into();
            let block = gear_api.api.blocks().at(block_hash).await?;
            let gear_block = GearBlock::from_subxt_block(&gear_api, block).await?;
            self.block_storage.add_block(&gear_api, &gear_block).await?;
            match tx.send(gear_block) {
                Ok(_) => {}
                Err(broadcast::error::SendError(_)) => {
                    log::error!("No active receivers for Gear block listener, stopping");
                    return Ok(false);
                }
            }

            // Update the last finalized block number
            last_finalized_block_number = Some(block_number);
            self.metrics.latest_block.set(block_number as i64);
        }

        Ok(true)
    }
}
