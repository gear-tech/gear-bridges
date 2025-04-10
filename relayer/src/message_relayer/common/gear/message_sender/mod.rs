use std::time::Duration;

use crate::{
    common,
    message_relayer::common::{EthereumSlotNumber, GSdkArgs, TxHashWithSlot},
};
use anyhow::anyhow;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use futures::executor::block_on;
use gclient::{GearApi, WSAddress};
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::{
    calls::{Action, ActionIo, Call},
    gclient::calls::GClientRemoting,
    Encode,
};

use tokio::sync::mpsc::UnboundedReceiver;
use utils_prometheus::{impl_metered_service, MeteredService};
use vft_manager_client::vft_manager::io::SubmitReceipt;

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
    historical_proxy_address: H256,
    receiver_address: H256,
    receiver_route: Vec<u8>,
    decode_reply: bool,

    waiting_checkpoint: Vec<TxHashWithSlot>,

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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        args: GSdkArgs,
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        historical_proxy_address: H256,
        receiver_address: H256,
        receiver_route: Vec<u8>,
        decode_reply: bool,
    ) -> Self {
        Self {
            args,
            suri,
            eth_api,
            beacon_client,
            historical_proxy_address,
            receiver_address,
            receiver_route,
            decode_reply,

            waiting_checkpoint: vec![],

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<TxHashWithSlot>,
        mut checkpoints: UnboundedReceiver<EthereumSlotNumber>,
    ) {
        let _ = tokio::task::spawn_blocking(move || {
            block_on(async move {
                let base_delay = Duration::from_secs(1);
                let mut attempts = 0;
                const MAX_ATTEMPTS: u32 = 5;
                loop {
                    match self.run_inner(&mut messages, &mut checkpoints).await {
                        Ok(_) => continue,
                        Err(err) => {
                            attempts += 1;
                            let delay = base_delay * 2u32.pow(attempts - 1);
                            log::error!(
                                "Gear message sender failed (attempt {}/{}): {}. Retrying in {:?}",
                                attempts,
                                MAX_ATTEMPTS,
                                err,
                                delay
                            );
                            if attempts >= MAX_ATTEMPTS {
                                log::error!("Max attempts reached, exiting...");
                                break;
                            }

                            tokio::time::sleep(delay).await;

                            if common::is_transport_error_recoverable(&err) {
                                self.eth_api = match self.eth_api.reconnect() {
                                    Ok(eth_api) => eth_api,
                                    Err(err) => {
                                        log::error!("Failed to reconnect to Ethereum API: {}", err);
                                        break;
                                    }
                                };
                            }
                        }
                    }
                }
            });
        })
        .await;
    }

    async fn run_inner(
        &mut self,
        messages: &mut UnboundedReceiver<TxHashWithSlot>,
        checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        let mut latest_checkpoint_slot = None;

        loop {
            while let Ok(checkpoint) = checkpoints.try_recv() {
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

            while let Ok(message) = messages.try_recv() {
                self.waiting_checkpoint.push(message);
            }

            if self.waiting_checkpoint.is_empty() {
                continue;
            }

            let gear_api = create_gclient_client(&self.args, &self.suri).await?;
            for i in (0..self.waiting_checkpoint.len()).rev() {
                if self.waiting_checkpoint[i].slot_number
                    <= latest_checkpoint_slot.unwrap_or_default()
                {
                    self.submit_message(&self.waiting_checkpoint[i], &gear_api)
                        .await?;
                    let _ = self.waiting_checkpoint.remove(i);
                }
            }

            self.update_balance_metric(&gear_api).await?;

            self.metrics
                .messages_waiting_checkpoint
                .set(self.waiting_checkpoint.len() as i64);
        }
    }

    async fn submit_message(
        &self,
        message: &TxHashWithSlot,
        gear_api: &GearApi,
    ) -> anyhow::Result<()> {
        let payload =
            compose_payload::compose(&self.beacon_client, &self.eth_api, message.tx_hash).await?;

        log::info!(
            "Sending message in gear_message_sender: tx_index={}, slot={}",
            payload.transaction_index,
            payload.proof_block.block.slot
        );

        let gas_limit_block = gear_api.block_gas_limit()?;
        // Use 95% of block gas limit for all extrinsics.
        let gas_limit = gas_limit_block / 100 * 95;

        let remoting = GClientRemoting::new(gear_api.clone());

        let mut proxy_service = HistoricalProxy::new(remoting.clone());

        let (_, receiver_reply) = proxy_service
            .redirect(
                payload.proof_block.block.slot,
                payload.encode(),
                self.receiver_address.into(),
                self.receiver_route.clone(),
            )
            .with_gas_limit(gas_limit)
            .send_recv(self.historical_proxy_address.into())
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to send message to historical proxy address: {:?}",
                    e
                )
            })?
            .map_err(|e| anyhow::anyhow!("Internal historical proxy error: {:?}", e))?;

        // TODO: Refactor this approach. #255
        if self.decode_reply {
            let reply = SubmitReceipt::decode_reply(&receiver_reply)
                .map_err(|e| anyhow::anyhow!("Failed to decode vft-manager reply: {:?}", e))?;

            match reply {
                Ok(_) => {}
                Err(vft_manager_client::Error::NotSupportedEvent) => {
                    log::warn!("Dropping message for {} as it's considered invalid by vft-manager(probably unsupported ERC20 token)", message.tx_hash);
                }
                Err(e) => {
                    anyhow::bail!("Internal vft-manager error: {:?}", e);
                }
            }
        } else {
            log::info!("Received reply: {}", hex::encode(&receiver_reply));
        }

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
