use crate::message_relayer::{common::GearBlock, eth_to_gear::api_provider::ApiProviderConnection};
use futures::StreamExt;
<<<<<<< HEAD
use gsdk::subscription::BlockEvents;
=======
use primitive_types::H256;
>>>>>>> 4a6b422 (relayer: add GearBlock::from_subxt_block)
use prometheus::IntGauge;
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct BlockListener {
    api_provider: ApiProviderConnection,

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
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self {
            api_provider,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        // Capacity for the channel. At the moment merkle-root relayer might lag behind
        // during proof generation or era sync, so we need to have enough capacity
        // to not drop any blocks. 14400 is how many blocks are produced in 1 era.
        const CAPACITY: usize = 14400;
        let (tx, _) = broadcast::channel(CAPACITY);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&tx2).await;
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

    async fn run_inner(&self, tx: &broadcast::Sender<GearBlock>) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();

<<<<<<< HEAD
=======
        for (block_hash, block_number) in unprocessed.drain(..) {
            log::trace!("Fetching unprocessed block #{block_number} (hash: {block_hash})");
            let block = gear_api.api.blocks().at(block_hash).await?;

            let gear_block = GearBlock::from_subxt_block(block).await?;

            match tx.send(gear_block) {
                Ok(_) => (),
                Err(broadcast::error::SendError(_)) => {
                    log::error!("No active receivers for Gear block listener, stopping");
                    return Ok(false);
                }
            }
        }

>>>>>>> 4a6b422 (relayer: add GearBlock::from_subxt_block)
        let mut finalized_blocks = gear_api.api.subscribe_finalized_blocks().await?;
        loop {
            match finalized_blocks.next().await {
                Some(Err(err)) => {
                    log::error!("Error receiving finalized block: {err}");
                    break Err(err);
                }

                Some(Ok(block)) => {
                    self.metrics.latest_block.set(block.number() as i64);

<<<<<<< HEAD
                    match tx.send(GearBlock::new(header, events)) {
=======
                    let block = GearBlock::from_subxt_block(block).await?;
                    self.block_storage.add_block(&block).await;

                    match tx.send(block) {
>>>>>>> 4a6b422 (relayer: add GearBlock::from_subxt_block)
                        Ok(_) => (),
                        Err(broadcast::error::SendError(_)) => {
                            log::error!("No active receivers for Gear block listener, stopping");
                            return Ok(false);
                        }
                    }
                    self.metrics.latest_block.inc();
                }

                None => break Ok(true),
            }
        }
    }
}
