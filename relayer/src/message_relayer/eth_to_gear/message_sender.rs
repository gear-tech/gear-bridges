use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use futures::executor::block_on;
use gclient::GearApi;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    IntCounter, IntGauge,
};
use sails_rs::{
    calls::{Action, ActionIo, Call},
    gclient::calls::GClientRemoting,
    Encode,
};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;
use vft_manager_client::vft_manager::io::SubmitReceipt;

use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

pub struct MessageSenderIo {
    requests_channel: UnboundedSender<Request>,
    responses_channel: UnboundedReceiver<Response>,
}

impl MessageSenderIo {
    pub fn new(
        requests_channel: UnboundedSender<Request>,
        responses_channel: UnboundedReceiver<Response>,
    ) -> Self {
        Self {
            requests_channel,
            responses_channel,
        }
    }

    pub fn send_message(
        &mut self,
        tx_uuid: Uuid,
        tx_hash: FixedBytes<32>,
        payload: EthToVaraEvent,
    ) -> bool {
        self.requests_channel
            .send(Request {
                tx_uuid,
                tx_hash,
                payload,
            })
            .inspect_err(|err| log::error!("Message sender failed: {err:?}"))
            .is_ok()
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses_channel.recv().await
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub payload: EthToVaraEvent,
    pub tx_hash: FixedBytes<32>,
    pub tx_uuid: Uuid,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub tx_uuid: Uuid,
    pub status: MessageStatus,
}

#[derive(Clone, Debug)]
pub enum MessageStatus {
    Success,
    Failure(String),
}

impl_metered_service!(
    struct Metrics {
        fee_payer_balance: IntGauge = IntGauge::new(
            "gear_message_sender_fee_payer_balance",
            "Balance of the fee payer account",
        ),

        total_gas_used: GenericCounter<AtomicU64> = GenericCounter::new(
            "gear_message_sender_total_gas_used",
            "Total gas used by gear message sender",
        ),
        min_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_min_gas_used",
            "Minimum gas used by gear message sender",
        ),
        max_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_max_gas_used",
            "Maximum gas used by gear message sender",
        ),
        last_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_last_gas_used",
            "Last gas used by gear message sender",
        ),
        total_submissions: IntCounter = IntCounter::new(
            "gear_message_sender_total_submissions",
            "Total number of messages sent to Gear",
        ),
    }
);

pub struct MessageSender {
    pub receiver_address: H256,
    pub receiver_route: Vec<u8>,
    pub historical_proxy_address: H256,
    pub api_provider: ApiProviderConnection,
    pub suri: String,
    pub last_request: Option<Request>,

    metrics: Metrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MessageSender {
    pub fn new(
        receiver_address: H256,
        receiver_route: Vec<u8>,
        historical_proxy_address: H256,
        api_provider: ApiProviderConnection,
        suri: String,
    ) -> Self {
        Self {
            receiver_address,
            receiver_route,
            historical_proxy_address,
            api_provider,
            suri,
            last_request: None,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self) -> MessageSenderIo {
        let (requests_tx, requests_rx) = unbounded_channel();
        let (responses_tx, responses_rx) = unbounded_channel();

        spawn_blocking(move || block_on(task(self, requests_rx, responses_tx)));

        MessageSenderIo::new(requests_tx, responses_rx)
    }

    async fn run_inner(
        &mut self,
        requests: &mut UnboundedReceiver<Request>,
        responses: &mut UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        if let Some(request) = self.last_request.take() {
            match self.process(responses, &gear_api, &request).await {
                Ok(should_continue) => {
                    if !should_continue {
                        return Ok(());
                    }
                }
                Err(err) => {
                    log::error!(
                        "Transaction {} failed (retrying): {err:?}",
                        request.tx_hash
                    );
                    self.last_request = Some(request);
                    return Err(err);
                }
            }
        }

        while let Some(request) = requests.recv().await {
            self.update_balance_metric(&gear_api).await?;

            match self.process(responses, &gear_api, &request).await {
                Ok(should_continue) => {
                    if !should_continue {
                        return Ok(());
                    }
                }
                Err(err) => {
                    self.last_request = Some(request);
                    return Err(err);
                }
            }
        }

        Ok(())
    }

    async fn process(
        &mut self,
        responses: &mut UnboundedSender<Response>,
        gear_api: &GearApi,
        request: &Request,
    ) -> anyhow::Result<bool> {
        let Request {
            tx_uuid,
            payload,
            tx_hash,
        } = request;
        let tx_uuid = *tx_uuid;
        let gas_limit_block = gear_api.block_gas_limit()?;
        let gas_limit = gas_limit_block / 100 * 95;

        let remoting = GClientRemoting::new(gear_api.clone());

        let mut proxy_service = HistoricalProxy::new(remoting);

        let (_, receiver_reply) = proxy_service
            .redirect(
                payload.proof_block.block.slot,
                payload.encode(),
                self.receiver_address.0.into(),
                self.receiver_route.clone(),
            )
            .with_gas_limit(gas_limit)
            .send_recv(self.historical_proxy_address.0.into())
            .await
            .map_err(|e| {
                let error = anyhow::anyhow!("Failed to send message: {e:?}");
                responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Failure(error.to_string()),
                    })
                    .unwrap_or_default();

                error
            })?
            .map_err(|e| {
                let error = anyhow::anyhow!("Failed to receive message: {e:?}");
                responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Failure(error.to_string()),
                    })
                    .unwrap_or_default();

                error
            })?;

        self.metrics.total_submissions.inc();
        self.metrics.last_gas_used.set(gas_limit);
        self.metrics.total_gas_used.inc_by(gas_limit);

        if self.metrics.min_gas_used.get() == 0 || gas_limit < self.metrics.min_gas_used.get() {
            self.metrics.min_gas_used.set(gas_limit);
        }

        if gas_limit > self.metrics.max_gas_used.get() {
            self.metrics.max_gas_used.set(gas_limit);
        }

        log::debug!("Received reply: {}", hex::encode(&receiver_reply));

        let reply = SubmitReceipt::decode_reply(&receiver_reply).map_err(|e| {
            let error = anyhow::anyhow!("Failed to decode reply: {e}");
            responses
                .send(Response {
                    tx_uuid,
                    status: MessageStatus::Failure(error.to_string()),
                })
                .unwrap_or_default();
            error
        })?;

        match reply {
            Ok(()) => {
                if responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Success,
                    })
                    .is_err()
                {
                    return Ok(false);
                }
            }

            Err(vft_manager_client::Error::AlreadyProcessed) => {
                log::warn!("Message for {tx_hash:?} is already processed, skipping...");
                if responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Success,
                    })
                    .is_err()
                {
                    return Ok(false);
                }
            }

            Err(vft_manager_client::Error::UnsupportedEthEvent) => {
                let message = format!("Dropping message for {tx_hash:?} as it's considered invalid by vft-manager (probably unsupported ERC20 token)");
                log::warn!("{message}");
                if responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Failure(message),
                    })
                    .is_err()
                {
                    return Ok(false);
                }
            }

            Err(e) => {
                let message = format!("Internal vft-manager error: {e:?}");

                if responses
                    .send(Response {
                        tx_uuid,
                        status: MessageStatus::Failure(message),
                    })
                    .is_err()
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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

async fn task(
    mut this: MessageSender,
    mut requests: UnboundedReceiver<Request>,
    mut responses: UnboundedSender<Response>,
) {
    loop {
        if requests.is_closed() || responses.is_closed() {
            log::warn!("Transaction manager connection terminated, exiting...");
            break;
        }

        match this.run_inner(&mut requests, &mut responses).await {
            Ok(()) => {
                log::warn!("Transaction manager connection terminated, exiting...");
                break;
            }

            Err(err) => {
                log::error!("Gear message sender got an error: {err:?}");
                loop {
                    match this.api_provider.reconnect().await {
                        Ok(()) => {
                            log::info!("Reconnected to Gear API");
                            break;
                        }

                        Err(err) => {
                            log::error!("Failed to reconnect to Gear API: {err:?}. Retrying in 5s...");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        }
    }
}
