use crate::{
    common,
    message_relayer::common::{
        ethereum::block_storage::UnprocessedBlockStorage, EthereumBlockNumber,
    },
};
use ethereum_client::PollingEthApi;
use prometheus::IntGauge;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::MissedTickBehavior,
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct BlockListener {
    eth_api: PollingEthApi,
    from_block: u64,
    storage: Arc<dyn UnprocessedBlockStorage>,

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
            "ethereum_block_listener_latest_block",
            "Latest ethereum block discovered by listener",
        ),
    }
}

impl BlockListener {
    pub fn new(
        eth_api: PollingEthApi,
        from_block: u64,
        storage: Arc<dyn UnprocessedBlockStorage>,
    ) -> Self {
        Self {
            eth_api,
            from_block,
            storage,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self) -> UnboundedReceiver<EthereumBlockNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::spawn(task(self, sender));

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &UnboundedSender<EthereumBlockNumber>,
    ) -> anyhow::Result<()> {
        self.metrics.latest_block.set(self.from_block as i64);

        self.replay_recovery_blocks(sender, "startup recovery")
            .await?;
        let mut supervisor_interval = tokio::time::interval(common::SUPERVISOR_INTERVAL);
        supervisor_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        supervisor_interval.tick().await;

        loop {
            tokio::select! {
                _ = supervisor_interval.tick() => {
                    self.replay_recovery_blocks(sender, "supervisor recovery").await?;
                }

                latest = self.eth_api.finalized_block() => {
                    let latest = latest?.header.number;
                    if latest >= self.from_block {
                        self.send_block_range(sender, self.from_block, latest).await?;
                        self.metrics.latest_block.set(latest as i64);
                    } else {
                        tokio::time::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2).await;
                    }
                }
            }
        }
    }

    async fn replay_recovery_blocks(
        &mut self,
        sender: &UnboundedSender<EthereumBlockNumber>,
        reason: &'static str,
    ) -> anyhow::Result<()> {
        let blocks = self.storage.unprocessed_blocks().await?;
        let latest = self.eth_api.finalized_block().await?.header.number;
        let lookback_from = latest.saturating_sub(u64::from(common::SUPERVISOR_LOOKBACK_BLOCKS));
        if let Some(block) = blocks.first().copied() {
            if block < lookback_from {
                log::warn!(
                    "Ethereum block listener {reason}: oldest unprocessed block #{block} is older than the supervisor lookback start #{lookback_from}, capping replay"
                );
            } else {
                log::info!(
                    "Ethereum block listener {reason}: unprocessed block #{block} is inside the supervisor lookback window"
                );
            }
        } else {
            log::info!("Ethereum block listener {reason}: no persisted unprocessed blocks found");
        }

        log::info!(
            "Ethereum block listener {reason}: replaying last {} finalized block(s) from #{lookback_from}",
            common::SUPERVISOR_LOOKBACK_BLOCKS,
        );
        self.send_block_range(sender, lookback_from, latest).await?;
        self.metrics.latest_block.set(latest as i64);
        Ok(())
    }

    async fn send_block_range(
        &mut self,
        sender: &UnboundedSender<EthereumBlockNumber>,
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<()> {
        if from_block > to_block {
            return Ok(());
        }

        for block in from_block..=to_block {
            if let Err(e) = self.storage.add_block(block).await {
                log::error!("Failed to add block {block} to storage: {e}");
            }
            sender.send(EthereumBlockNumber(block))?;
            self.from_block = block.saturating_add(1);
        }
        Ok(())
    }
}

async fn task(mut this: BlockListener, sender: UnboundedSender<EthereumBlockNumber>) {
    loop {
        let result = this.run_inner(&sender).await;
        let Err(e) = result else {
            continue;
        };

        log::error!("Ethereum block listener failed: {e:?}");
        if !common::is_transport_error_recoverable(&e) {
            log::error!("Non recoverable error, exiting.");
            return;
        }

        tokio::time::sleep(Duration::from_secs(30)).await;

        match this.eth_api.reconnect().await {
            Ok(api) => {
                this.eth_api = api;
            }
            Err(e) => {
                log::error!("Failed to reconnect to Ethereum API: {e}. Exiting...");
                return;
            }
        }
    }
}
