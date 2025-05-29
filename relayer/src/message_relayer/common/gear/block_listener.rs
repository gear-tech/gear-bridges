use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;
use futures::StreamExt;
use gsdk::config::Header;
use prometheus::IntGauge;
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct BlockListener {
    api_provider: ApiProviderConnection,
    from_block: u32,

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
    pub fn new(api_provider: ApiProviderConnection, from_block: u32) -> Self {
        Self {
            api_provider,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<Header>; RECEIVER_COUNT] {
        let (tx, _) = broadcast::channel(RECEIVER_COUNT);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            let mut current_block = self.from_block;
            loop {
                let res = self.run_inner(&tx2, &mut current_block).await;
                if let Err(err) = res {
                    log::error!("Gear block listener failed: {}", err);

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
        });

        (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    async fn run_inner(
        &self,
        tx: &broadcast::Sender<Header>,
        current_block: &mut u32,
    ) -> anyhow::Result<()> {
        self.metrics.latest_block.set(*current_block as i64);
        let gear_api = self.api_provider.client();

        let mut finalized_blocks = gear_api.api.subscribe_finalized_blocks().await?;
        loop {
            match finalized_blocks.next().await {
                Some(Err(err)) => {
                    log::error!("Error receiving finalized block: {}", err);
                    break Err(err);
                }

                Some(Ok(block)) => {
                    *current_block = block.number() + 1;
                    match tx.send(block.header().clone()) {
                        Ok(_) => (),
                        Err(broadcast::error::SendError(_)) => {
                            log::error!("No active receivers for Gear block listener, stopping");
                            return Ok(());
                        }
                    }
                    self.metrics.latest_block.inc();
                }

                None => break Ok(()),
            }
        }
    }
}
