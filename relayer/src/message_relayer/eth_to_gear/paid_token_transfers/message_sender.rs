use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use futures::executor::block_on;
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

use crate::{
    common::{BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
};

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

    pub fn run(mut self) -> MessageSenderIo {
        let (requests_tx, mut requests_rx) = unbounded_channel();
        let (mut responses_tx, responses_rx) = unbounded_channel();

        spawn_blocking(move || {
            block_on(async move {
                let mut attempts = 0;
                loop {
                    if requests_rx.is_closed() || responses_tx.is_closed() {
                        log::warn!("Transaction manager connection terminated, exiting...");
                        break;
                    }

                    match self.run_inner(&mut requests_rx, &mut responses_tx).await {
                        Ok(()) => {
                            log::warn!("Transaction manager connection terminated, exiting...");
                            break;
                        }

                        Err(err) => {
                            log::error!("Gear message sender got an error (attempt {attempts}/{MAX_RETRIES}): {err:?}");
                            attempts += 1;

                            if attempts >= MAX_RETRIES {
                                log::error!("Max retries reached, terminating Gear message sender");
                                break;
                            }
                            let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);

                            tokio::time::sleep(delay).await;

                            match self.api_provider.reconnect().await {
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
            })
        });

        MessageSenderIo {
            requests: requests_tx,
            responses: responses_rx,
        }
    }

    async fn run_inner(
        &mut self,
        requests: &mut UnboundedReceiver<Message>,
        responses: &mut UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        while let Some(Message {
            tx_uuid,
            tx_hash,
            payload,
        }) = requests.recv().await
        {
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
                            status: SendStatus::Failure(error.to_string()),
                        })
                        .unwrap_or_default();

                    error
                })?
                .map_err(|e| {
                    let error = anyhow::anyhow!("Failed to receive message: {e:?}");
                    responses
                        .send(Response {
                            tx_uuid,
                            status: SendStatus::Failure(error.to_string()),
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
                        status: SendStatus::Failure(error.to_string()),
                    })
                    .unwrap_or_default();
                error
            })?;

            match reply {
                Ok(()) => {
                    if responses
                        .send(Response {
                            tx_uuid,
                            status: SendStatus::Success,
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }

                Err(vft_manager_client::Error::AlreadyProcessed) => {
                    log::warn!("Message for {tx_hash:?} is already processed, skipping...");
                    if responses
                        .send(Response {
                            tx_uuid,
                            status: SendStatus::Success,
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }

                Err(vft_manager_client::Error::NotSupportedEvent) => {
                    let message = format!("Dropping message for {tx_hash:?} as it's considered invalid by vft-manager (probably unsupported ERC20 token)");
                    log::warn!("{message}");
                    if responses
                        .send(Response {
                            tx_uuid,
                            status: SendStatus::Failure(message),
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }

                Err(e) => {
                    let message = format!("Internal vft-manager error: {e:?}");

                    if responses
                        .send(Response {
                            tx_uuid,
                            status: SendStatus::Failure(message),
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }
        }

        Ok(())
    }
}

pub struct MessageSenderIo {
    requests: UnboundedSender<Message>,
    responses: UnboundedReceiver<Response>,
}

impl MessageSenderIo {
    pub fn new(requests: UnboundedSender<Message>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
            responses,
        }
    }

    pub fn send_message(
        &mut self,
        tx_uuid: Uuid,
        tx_hash: FixedBytes<32>,
        payload: EthToVaraEvent,
    ) -> bool {
        self.requests
            .send(Message {
                tx_uuid,
                tx_hash,
                payload,
            })
            .is_ok()
    }

    pub async fn receive_response(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub payload: EthToVaraEvent,
    pub tx_hash: FixedBytes<32>,
    pub tx_uuid: Uuid,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub tx_uuid: Uuid,
    pub status: SendStatus,
}

#[derive(Clone, Debug)]
pub enum SendStatus {
    Success,
    Failure(String),
}
