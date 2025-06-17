use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::common::{EthereumSlotNumber, TxHashWithSlot},
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
};
use checkpoint_light_client_client::{traits::ServiceState as _, Order, ServiceState};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use futures::{
    executor::block_on,
    future::{self, Either},
    pin_mut,
};
use gclient::GearApi;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::{
    calls::{Action, ActionIo, Call, Query},
    gclient::calls::GClientRemoting,
    Encode,
};

use tokio::{sync::mpsc::UnboundedReceiver, time::Duration};
use utils_prometheus::{impl_metered_service, MeteredService};
use vft_manager_client::vft_manager::io::SubmitReceipt;

pub mod compose_payload;

pub struct MessageSender {
    api_provider: ApiProviderConnection,
    suri: String,
    eth_api: EthApi,
    beacon_client: BeaconClient,
    historical_proxy_address: H256,
    checkpoint_light_client_address: H256,
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
        api_provider: ApiProviderConnection,
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        historical_proxy_address: H256,
        checkpoint_light_client_address: H256,
        receiver_address: H256,
        receiver_route: Vec<u8>,
        decode_reply: bool,
    ) -> Self {
        Self {
            api_provider,
            suri,
            eth_api,
            beacon_client,
            historical_proxy_address,
            checkpoint_light_client_address,
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
                let mut attempts = 0;

                loop {
                    match run_inner(&mut self, &mut messages, &mut checkpoints).await {
                        Ok(_) => break,
                        Err(err) => {
                            log::error!("Gear message sender failed with: {err}");

                            attempts += 1;
                            let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                            log::error!(
                                "Gear message sender failed (attempt {attempts}/{MAX_RETRIES}): {err}. Retrying in {delay:?}"
                            );
                            if attempts >= MAX_RETRIES {
                                log::error!("Max attempts reached, exiting...");
                                break;
                            }

                            tokio::time::sleep(delay).await;

                            match self.api_provider.reconnect().await {
                                Ok(()) => {
                                    log::info!("Gear message sender reconnected");
                                }
                                Err(err) => {
                                    log::error!("Gear message sender unable to reconnect: {err}");
                                    return;
                                }
                            }

                            if common::is_transport_error_recoverable(&err) {
                                self.eth_api = match self.eth_api.reconnect().await {
                                    Ok(eth_api) => eth_api,
                                    Err(err) => {
                                        log::error!("Failed to reconnect to Ethereum API: {err}");
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

    async fn submit_message(
        &self,
        message: &TxHashWithSlot,
        gear_api: &GearApi,
    ) -> anyhow::Result<()> {
        let payload = compose_payload::compose(
            &self.beacon_client,
            gear_api,
            &self.eth_api,
            message.tx_hash,
            self.historical_proxy_address.into(),
        )
        .await?;

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
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to receive reply from historical proxy address: {:?}",
                    e
                )
            })?;

        // TODO: Refactor this approach. #255
        log::debug!("Received reply: {}", hex::encode(&receiver_reply));
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
        }

        Ok(())
    }

    async fn update_balance_metric(&self, gear_api: &GearApi) -> anyhow::Result<()> {
        let balance = gear_api
            .total_balance(gear_api.account_id())
            .await
            .map_err(|e| anyhow::anyhow!("Unable to get total balance: {e:?}"))?;

        let balance = balance / 1_000_000_000_000;
        let balance: i64 = balance.try_into().unwrap_or(i64::MAX);

        self.metrics.fee_payer_balance.set(balance);

        Ok(())
    }
}

async fn run_inner(
    self_: &mut MessageSender,
    messages: &mut UnboundedReceiver<TxHashWithSlot>,
    checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
) -> anyhow::Result<()> {
    let gear_api = self_.api_provider.gclient_client(&self_.suri)?;

    let remoting = GClientRemoting::new(gear_api.clone());
    let service = ServiceState::new(remoting);
    let state = service
        .get(Order::Reverse, 0, 1)
        .recv(self_.checkpoint_light_client_address.into())
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to get service state from checkpoint light client: {:?}",
                e
            )
        })?;
    assert!(state.checkpoints.len() <= 1);

    let mut latest_checkpoint_slot = state.checkpoints.first().map(|c| EthereumSlotNumber(c.0));

    loop {
        self_.update_balance_metric(&gear_api).await?;

        let recv_messages = messages.recv();
        pin_mut!(recv_messages);

        let recv_checkpoints = checkpoints.recv();
        pin_mut!(recv_checkpoints);

        match future::select(recv_messages, recv_checkpoints).await {
            Either::Left((None, _)) => {
                log::info!("Channel with messages closed. Exiting");
                return Ok(());
            }

            Either::Right((None, _)) => {
                log::info!("Channel with checkpoints closed. Exiting");
                return Ok(());
            }

            Either::Left((Some(message), _)) => self_.waiting_checkpoint.push(message),

            Either::Right((Some(checkpoint), _)) => {
                if latest_checkpoint_slot.unwrap_or_default() < checkpoint {
                    latest_checkpoint_slot = Some(checkpoint);
                } else {
                    log::error!(
                        "Received checkpoints not in sequential order. \
                    Previously found checkpoint: {latest_checkpoint_slot:?} and new checkpoint is {checkpoint}"
                    );
                }
            }
        }

        if self_.waiting_checkpoint.is_empty() {
            log::info!("There are no waiting checkpoints.");
            tokio::time::sleep(Duration::from_millis(300)).await;

            continue;
        }

        for i in (0..self_.waiting_checkpoint.len()).rev() {
            if self_.waiting_checkpoint[i].slot_number <= latest_checkpoint_slot.unwrap_or_default()
            {
                self_
                    .submit_message(&self_.waiting_checkpoint[i], &gear_api)
                    .await?;
                let _ = self_.waiting_checkpoint.remove(i);
            }
        }

        self_.update_balance_metric(&gear_api).await?;

        self_
            .metrics
            .messages_waiting_checkpoint
            .set(self_.waiting_checkpoint.len() as i64);
    }
}
