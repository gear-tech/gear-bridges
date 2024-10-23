use std::sync::mpsc::{channel, Receiver, Sender};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use checkpoint_light_client_io::meta::{Order, State, StateRequest};

use crate::message_relayer::common::{EthereumSlotNumber, GearBlockNumber};

pub struct CheckpointsExtractor {
    checkpoint_light_client_address: H256,

    gear_api: GearApi,

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
    pub fn new(gear_api: GearApi, checkpoint_light_client_address: H256) -> Self {
        Self {
            checkpoint_light_client_address,
            gear_api,
            latest_checkpoint: None,
            metrics: Metrics::new(),
        }
    }

    pub fn run(mut self, blocks: Receiver<GearBlockNumber>) -> Receiver<EthereumSlotNumber> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Checkpoints extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &Sender<EthereumSlotNumber>,
        blocks: &Receiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block.0, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &mut self,
        block: u32,
        sender: &Sender<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        let request = StateRequest {
            order: Order::Reverse,
            index_start: 0,
            count: 1,
        }
        .encode();

        let state = self
            .gear_api
            .api
            .read_state(
                self.checkpoint_light_client_address,
                request,
                Some(block_hash),
            )
            .await?;

        let state = hex::decode(&state[2..])?;
        let state = State::decode(&mut &state[..])?;

        assert!(state.checkpoints.len() <= 1);

        let latest_checkpoint = state.checkpoints.first();

        match (latest_checkpoint, self.latest_checkpoint) {
            (None, None) => {}
            (None, Some(_)) => {
                panic!(
                    "Invalid state detected: checkpoint-light-client program contains no checkpoints \
                    but there's one in checkpoints extractor state"
                );
            }
            (Some(checkpoint), None) => {
                self.latest_checkpoint = Some(EthereumSlotNumber(checkpoint.0));

                self.metrics.latest_checkpoint_slot.set(checkpoint.0 as i64);

                log::info!("First checkpoint discovered: {}", checkpoint.0);

                sender.send(EthereumSlotNumber(checkpoint.0))?;
            }
            (Some(latest), Some(stored)) => {
                if latest.0 > stored.0 {
                    self.metrics.latest_checkpoint_slot.set(latest.0 as i64);

                    let latest = EthereumSlotNumber(latest.0);

                    self.latest_checkpoint = Some(latest);

                    log::info!("New checkpoint discovered: {}", latest.0);

                    sender.send(latest)?;
                }
            }
        }

        Ok(())
    }
}
