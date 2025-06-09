use crate::message_relayer::common::{gear::block_listener::GearBlock, EthereumSlotNumber};
use checkpoint_light_client_client::{
    service_replay_back::events::ServiceReplayBackEvents,
    service_sync_update::events::ServiceSyncUpdateEvents,
};
use primitive_types::H256;
use prometheus::IntGauge;

use sails_rs::events::EventIo;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct CheckpointsExtractor {
    checkpoint_light_client_address: H256,

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

pub fn checkpoints_for_block(block: &GearBlock, program_address: H256) -> Vec<(u64, H256)> {
    let mut checkpoints = block
        .user_message_sent_events(program_address, H256::zero())
        .filter_map(|payload| {
            ServiceReplayBackEvents::decode_event(payload)
                .ok()
                .map(
                    |ServiceReplayBackEvents::NewCheckpoint {
                         slot,
                         tree_hash_root,
                     }| (slot, tree_hash_root),
                )
                .or_else(|| {
                    ServiceSyncUpdateEvents::decode_event(payload).ok().map(
                        |ServiceSyncUpdateEvents::NewCheckpoint {
                             slot,
                             tree_hash_root,
                         }| (slot, tree_hash_root),
                    )
                })
        })
        .collect::<Vec<_>>();
    checkpoints.sort_by(|a, b| a.0.cmp(&b.0));
    checkpoints
}

impl CheckpointsExtractor {
    pub fn new(checkpoint_light_client_address: H256) -> Self {
        Self {
            checkpoint_light_client_address,

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: Receiver<GearBlock>,
    ) -> UnboundedReceiver<EthereumSlotNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            let res = self.run_inner(&sender, &mut blocks).await;
            if let Err(err) = res {
                log::error!("Checkpoints extractor failed: {err}");
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
        let checkpoints = checkpoints_for_block(&block, self.checkpoint_light_client_address);
        for checkpoint in checkpoints {
            self.metrics.latest_checkpoint_slot.set(checkpoint.0 as i64);
            log::info!("New checkpoint discovered: {}", checkpoint.0);
            sender.send(EthereumSlotNumber(checkpoint.0))?;
        }

        Ok(())
    }
}
