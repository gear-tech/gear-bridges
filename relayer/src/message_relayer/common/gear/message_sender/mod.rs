use std::sync::mpsc::Receiver;

use anyhow::anyhow;
use ethereum_client::EthApi;
use futures::executor::block_on;
use gclient::{GearApi, WSAddress};
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::{
    calls::{Action, Call},
    gclient::calls::GClientRemoting,
};

use erc20_relay_client::{traits::Erc20Relay as _, Erc20Relay};
use ethereum_beacon_client::BeaconClient;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::common::{EthereumSlotNumber, GSdkArgs, TxHashWithSlot};

mod compose_payload;

async fn create_gclient_client(args: &GSdkArgs, suri: &str) -> anyhow::Result<GearApi> {
    GearApi::builder()
        .retries(args.vara_rpc_retries)
        .suri(suri)
        .build(WSAddress::new(&args.vara_domain, args.vara_port))
        .await
        .map_err(|e| anyhow!("Failed to build GearApi: {e:?}"))
}

pub struct MessageSender {
    args: GSdkArgs,
    suri: String,
    eth_api: EthApi,
    beacon_client: BeaconClient,

    ethereum_event_client_address: H256,

    metrics: Metrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        messages_waiting_checkpoint: IntGauge = IntGauge::new(
            "gear_message_sender_messages_waiting_checkpoint",
            "Amount of messages waiting for corresponding checkpoint",
        ),
        messages_waiting_finality: IntGauge = IntGauge::new(
            "gear_message_sender_messages_waiting_finality",
            "Amount of messages waiting for finality on gear",
        ),
        fee_payer_balance: IntGauge = IntGauge::new(
            "gear_message_sender_fee_payer_balance",
            "Transaction fee payer balance",
        )
    }
}

impl MessageSender {
    pub fn new(
        args: GSdkArgs,
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        ethereum_event_client_address: H256,
    ) -> Self {
        Self {
            args,
            suri,
            eth_api,
            beacon_client,

            ethereum_event_client_address,

            metrics: Metrics::new(),
        }
    }

    pub fn run(
        self,
        messages: Receiver<TxHashWithSlot>,
        checkpoints: Receiver<EthereumSlotNumber>,
    ) {
        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&messages, &checkpoints));
            if let Err(err) = res {
                log::error!("Gear message sender failed: {}", err);
            }
        });
    }

    async fn run_inner(
        &self,
        messages: &Receiver<TxHashWithSlot>,
        checkpoints: &Receiver<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        let mut waiting_checkpoint: Vec<TxHashWithSlot> = vec![];

        let mut latest_checkpoint_slot = None;

        loop {
            for checkpoint in checkpoints.try_iter() {
                if latest_checkpoint_slot.unwrap_or_default() < checkpoint {
                    latest_checkpoint_slot = Some(checkpoint);
                } else {
                    log::error!(
                        "Received checkpoints not in sequential order. \
                        Previously found checkpoint: {:?} and new checkpoint is {}",
                        latest_checkpoint_slot,
                        checkpoint
                    );
                }
            }

            for message in messages.try_iter() {
                waiting_checkpoint.push(message);
            }

            if waiting_checkpoint.is_empty() {
                continue;
            }

            let gear_api = create_gclient_client(&self.args, &self.suri).await?;
            for i in (0..waiting_checkpoint.len()).rev() {
                if waiting_checkpoint[i].slot_number <= latest_checkpoint_slot.unwrap_or_default() {
                    self.submit_message(&waiting_checkpoint[i], &gear_api)
                        .await?;
                    let _ = waiting_checkpoint.remove(i);
                }
            }

            self.update_balance_metric(&gear_api).await?;

            self.metrics
                .messages_waiting_checkpoint
                .set(waiting_checkpoint.len() as i64);
        }
    }

    async fn submit_message(
        &self,
        message: &TxHashWithSlot,
        gear_api: &GearApi,
    ) -> anyhow::Result<()> {
        let message =
            compose_payload::compose(&self.beacon_client, &self.eth_api, message.tx_hash).await?;

        log::info!(
            "Sending message in gear_message_sender: tx_index={}, slot={}",
            message.transaction_index,
            message.proof_block.block.slot
        );

        let gas_limit_block = gear_api.block_gas_limit()?;
        // Use 95% of block gas limit for all extrinsics.
        let gas_limit = gas_limit_block / 100 * 95;

        let remoting = GClientRemoting::new(gear_api.clone());

        let mut erc20_service = Erc20Relay::new(remoting.clone());

        erc20_service
            .relay(message)
            .with_gas_limit(gas_limit)
            .send_recv(self.ethereum_event_client_address.into())
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send message to ethereum event client"))?
            .map_err(|_| anyhow::anyhow!("Internal ethereum event clint error"))?;

        Ok(())
    }

    async fn update_balance_metric(&self, gear_api: &GearApi) -> anyhow::Result<()> {
        let balance = gear_api.total_balance(gear_api.account_id()).await?;

        let balance = balance / 1_000_000_000_000;
        let balance: i64 = balance.try_into().unwrap_or(i64::MAX);

        self.metrics.fee_payer_balance.set(balance);

        Ok(())
    }
}
