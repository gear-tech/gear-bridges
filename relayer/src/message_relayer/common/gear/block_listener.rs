use crate::message_relayer::common::{
    gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
    GearBlock,
};
use futures::StreamExt;
use gear_common::{
    retry_api::{retry_n, Api, GearApiBuilder},
    ApiProviderConnection,
};
use gear_rpc_client::GearApi;
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
            let UnprocessedBlocks {
                last_block,
                first_block,
                blocks: _,
            } = self.block_storage.unprocessed_blocks().await;
            let mut api = Api::new(self.api_provider.clone());
            let mut unprocessed = match api
                .retry_n(
                    |gear_api| async {
                        let mut unprocessed = Vec::new();

                        if let Some(from_block) = last_block.or(first_block) {
                            let latest_finalized_block =
                                match gear_api.latest_finalized_block().await {
                                    Ok(block) => block,
                                    Err(err) => {
                                        log::error!("Failed to get latest finalized block: {err}");
                                        return Err(err);
                                    }
                                };

                            let latest_finalized_block_number = gear_api
                                .block_hash_to_number(latest_finalized_block)
                                .await?;

                            for block in from_block.1..=latest_finalized_block_number {
                                let hash = if block == latest_finalized_block_number {
                                    latest_finalized_block
                                } else {
                                    match gear_api.block_number_to_hash(block).await {
                                        Ok(hash) => hash,
                                        Err(err) => {
                                            log::error!(
                                            "Failed to get block hash for number {block}: {err}"
                                        );
                                            continue;
                                        }
                                    }
                                };
                                unprocessed.push((hash, block));
                            }
                        }

                        return Ok(unprocessed);
                    },
                    3,
                    GearApiBuilder,
                )
                .await
            {
                Ok(blocks) => blocks,
                Err(err) => {
                    log::error!("Failed to fetch unprocessed blocks: {err}");
                    return;
                }
            };

            let res = self.run_inner(&tx2, &mut unprocessed).await;
            match res {
                Ok(()) => {
                    log::info!("Gear block listener stopped due to no active receivers");
                }

                Err(err) => {
                    log::error!("Gear block listener failed: {err}");
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
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        mut unprocessed: &Vec<(H256, u32)>,
    ) -> anyhow::Result<()> {
        let mut api = Api::new(self.api_provider.clone());
        /*self.api
        .retry_n(
            |gear_api| async {
                for &(block_hash, block_number) in unprocessed.iter() {
                    log::trace!(
                        "Fetching unprocessed block #{block_number} (hash: {block_hash})"
                    );
                    let block = gear_api.api.blocks().at(block_hash).await?;

                    let gear_block = GearBlock::from_subxt_block(block).await?;

                    match tx.send(gear_block) {
                        Ok(_) => (),
                        Err(broadcast::error::SendError(_)) => {
                            log::error!(
                                "No active receivers for Gear block listener, stopping"
                            );
                            return Ok(());
                        }
                    }
                }

                Ok(())
            },
            1,
            GearApiBuilder,
        )
        .await?;*/

        api.retry_n(
            |gear_api| async {
                let mut finalized_blocks = gear_api.api.subscribe_finalized_blocks().await?;
                loop {
                    match finalized_blocks.next().await {
                        Some(Err(err)) => {
                            log::error!("Error receiving finalized block: {err}");
                            break Err(err);
                        }

                        Some(Ok(block)) => {
                            self.metrics.latest_block.set(block.number() as i64);

                            let block = GearBlock::from_subxt_block(block).await?;
                            self.block_storage.add_block(&block).await;

                            match tx.send(block) {
                                Ok(_) => (),
                                Err(broadcast::error::SendError(_)) => {
                                    log::error!(
                                        "No active receivers for Gear block listener, stopping"
                                    );
                                    return Ok(());
                                }
                            }
                            self.metrics.latest_block.inc();
                        }

                        None => break Ok(()),
                    }
                }
            },
            5,
            GearApiBuilder,
        )
        .await
        .into()
    }
}
