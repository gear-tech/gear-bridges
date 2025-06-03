use crate::message_relayer::{
    common::{gear::block_listener::GearBlock, EthereumSlotNumber},
    eth_to_gear::api_provider::ApiProviderConnection,
};
use primitive_types::H256;
use prometheus::IntGauge;

use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct CheckpointsExtractor {
    checkpoint_light_client_address: H256,

    api_provider: ApiProviderConnection,

    latest_checkpoint: Option<EthereumSlotNumber>,

    metrics: Metrics,
}

impl MeteredService for CheckpointsExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_checkpoint_slot: IntGauge = IntGauge::new(
            "checkpoint_extractor_latest_checkpoint_slot",
            "Latest slot found in checkpoint light client program state",
        ),
    }
}

impl CheckpointsExtractor {
    pub fn new(api_provider: ApiProviderConnection, checkpoint_light_client_address: H256) -> Self {
        Self {
            checkpoint_light_client_address,
            api_provider,
            latest_checkpoint: None,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: Receiver<GearBlock>,
    ) -> UnboundedReceiver<EthereumSlotNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    log::error!("Checkpoints extractor failed: {}", err);
                    match self.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Checkpoints extractor reconnected");
                        }
                        Err(err) => {
                            log::error!("Checkpoints extractor unable to reconnect: {err}");
                            return;
                        }
                    };
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &UnboundedSender<EthereumSlotNumber>,
        blocks: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<()> {
        loop {
            match blocks.recv().await {
                Ok(block) => self.process_block_events(block, sender).await?,
                Err(RecvError::Closed) => {
                    log::warn!("Checkpoints extractor channel closed, exiting");
                    return Ok(());
                }

                Err(RecvError::Lagged(_)) => {
                    log::warn!("Checkpoints extractor channel lagged behind, trying again");
                    continue;
                }
            }
        }
    }

    async fn process_block_events(
        &mut self,
        block: GearBlock,
        sender: &UnboundedSender<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        for checkpoint in block.new_checkpoints(self.checkpoint_light_client_address) {
            match self.latest_checkpoint {
                Some(stored) if checkpoint.0 > stored.0 => {
                    self.metrics.latest_checkpoint_slot.set(checkpoint.0 as i64);
                    self.latest_checkpoint = Some(EthereumSlotNumber(checkpoint.0));

                    log::info!("New checkpoint discovered: {}", checkpoint.0);

                    sender.send(EthereumSlotNumber(checkpoint.0))?;
                }

                // checkpoint is older than the stored one
                Some(_) => continue,

                None => {
                    self.latest_checkpoint = Some(EthereumSlotNumber(checkpoint.0));
                    self.metrics.latest_checkpoint_slot.set(checkpoint.0 as i64);

                    log::info!("First checkpoint discovered: {}", checkpoint.0);

                    sender.send(EthereumSlotNumber(checkpoint.0))?;
                }
            }
        }

        Ok(())
    }
}
