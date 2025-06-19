use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use futures::executor::block_on;
use gclient::GearApi;
use historical_proxy_client::traits::HistoricalProxy as _;
use historical_proxy_client::HistoricalProxy;
use primitive_types::H256;
use sails_rs::{
    calls::{Action, ActionIo, Call},
    gclient::calls::GClientRemoting,
    Encode,
};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
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

pub struct MessageSender {
    pub receiver_address: H256,
    pub receiver_route: Vec<u8>,
    pub historical_proxy_address: H256,
    pub api_provider: ApiProviderConnection,
    pub suri: String,
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

        while let Some(request) = requests.recv().await {
            if !self.process(responses, &gear_api, request).await? {
                return Ok(());
            }
        }

        Ok(())
    }

    async fn process(
        &mut self,
        responses: &mut UnboundedSender<Response>,
        gear_api: &GearApi,
        request: Request,
    ) -> anyhow::Result<bool> {
        let Request {
            tx_uuid,
            payload,
            tx_hash,
        } = request;
        let gas_limit_block = gear_api.block_gas_limit()?;
        let gas_limit = gas_limit_block / 100 * 95;

        let remoting = GClientRemoting::new(gear_api.clone());

        let mut proxy_service = HistoricalProxy::new(remoting);

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
        log::debug!("Received reply: {}", hex::encode(&receiver_reply));

        let reply = SubmitReceipt::decode_reply(&receiver_reply).map_err(|e| {
            let error = anyhow::anyhow!("Failed to decode reply: {}", e);
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

            Err(vft_manager_client::Error::NotSupportedEvent) => {
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
                match this.api_provider.reconnect().await {
                    Ok(()) => {
                        log::info!("Reconnected to Gear API");
                    }

                    Err(err) => {
                        log::error!("Failed to reconnect to Gear API: {err:?}");
                        break;
                    }
                }
            }
        }
    }
}
