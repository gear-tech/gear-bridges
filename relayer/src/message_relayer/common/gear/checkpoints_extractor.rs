use crate::message_relayer::common::{EthereumSlotNumber, GSdkArgs, GearBlockNumber};
use anyhow::anyhow;
use checkpoint_light_client_client::Order;
use gear_core::message::ReplyCode;
use gear_rpc_client::GearApi;
use parity_scale_codec::Decode;
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::calls::*;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct CheckpointsExtractor {
    checkpoint_light_client_address: H256,

    args: GSdkArgs,

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
        restarts: IntGauge = IntGauge::new(
            "checkpoint_extractor_restarts",
            "Number of restarts of the checkpoint extractor due to errors",
        ),
    }
}

impl CheckpointsExtractor {
    pub fn new(args: GSdkArgs, checkpoint_light_client_address: H256) -> Self {
        Self {
            checkpoint_light_client_address,
            args,
            latest_checkpoint: None,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: UnboundedReceiver<GearBlockNumber>,
    ) -> UnboundedReceiver<EthereumSlotNumber> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    log::error!("Checkpoints extractor failed: {}", err);
                    self.metrics.restarts.inc();
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &UnboundedSender<EthereumSlotNumber>,
        blocks: &mut UnboundedReceiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        let gear_api = GearApi::new(
            &self.args.vara_domain,
            self.args.vara_port,
            self.args.vara_rpc_retries,
        )
        .await?;

        loop {
            while let Ok(block) = blocks.try_recv() {
                self.process_block_events(&gear_api, block.0, sender)
                    .await?;
            }
        }
    }

    async fn process_block_events(
        &mut self,
        gear_api: &GearApi,
        block: u32,
        sender: &UnboundedSender<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        use checkpoint_light_client_client::service_state::io;

        let block_hash = gear_api.block_number_to_hash(block).await?;

        let payload = io::Get::encode_call(Order::Reverse, 0, 1);
        let api = gclient::GearApi::from(gear_api.api.clone());
        let origin = H256::from_slice(api.account_id().as_ref());
        let gas_limit = api.block_gas_limit()?;

        let reply_info = api
            .calculate_reply_for_handle_at(
                Some(origin),
                self.checkpoint_light_client_address.into(),
                payload,
                gas_limit,
                0,
                Some(block_hash),
            )
            .await?;

        let state: <io::Get as ActionIo>::Reply = match reply_info.code {
            ReplyCode::Success(_) => Decode::decode(&mut &reply_info.payload[..])?,
            ReplyCode::Error(reason) => Err(anyhow!("Failed to query state, reason: {reason:?}"))?,
            ReplyCode::Unsupported => Err(anyhow!("Failed to query state"))?,
        };

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
