use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use futures::executor::block_on;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use sails_rs::{
    calls::{Action, ActionIo, Call},
    gclient::calls::GClientRemoting,
    Encode,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;
use vft_manager_client::vft_manager::io::SubmitReceipt;

use crate::common::BASE_RETRY_DELAY;
use crate::common::MAX_RETRIES;
use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

pub struct MessageSender {
    pub vft_mananager_address: H256,
    pub historical_proxy_address: H256,
    pub api: ApiProviderConnection,
    pub suri: String,
}

impl MessageSender {
    pub fn new(
        vft_mananager_address: H256,
        historical_proxy_address: H256,
        api: ApiProviderConnection,
        suri: String,
    ) -> Self {
        Self {
            vft_mananager_address,
            historical_proxy_address,
            api,
            suri,
        }
    }

    pub fn run(mut self) -> (UnboundedSender<Message>, UnboundedReceiver<Response>) {
        let (msg_sender, mut msg_receiver) = unbounded_channel();
        let (mut response_sender, response_receiver) = unbounded_channel();

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                let mut attempts = 0;
                loop {
                    if msg_receiver.is_closed() || response_sender.is_closed() {
                        log::info!("MessageSender channels are closed, terminating");
                        break;
                    }

                    match self
                        .run_inner(&mut msg_receiver, &mut response_sender)
                        .await
                    {
                        Ok(()) => {
                            log::info!("MessageSender terminating");
                            break;
                        }

                        Err(e) => {
                            attempts += 1;
                            let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                            log::error!("Gear message sender failed (attempt {attempts}/{MAX_RETRIES}) with: {e:?}");

                            if attempts >= MAX_RETRIES {
                                log::error!("Max retries reached, terminating Gear message sender");
                                break;
                            }

                            tokio::time::sleep(delay).await;

                            match self.api.reconnect().await {
                                Ok(()) => {
                                    log::info!("Reconnected to API provider");
                                }
                                Err(e) => {
                                    log::error!("Failed to reconnect to API provider: {e}");
                                    return;
                                }
                            }
                        }
                    }
                }
            })
        });

        (msg_sender, response_receiver)
    }

    async fn run_inner(
        &mut self,
        msg_receiver: &mut UnboundedReceiver<Message>,
        response_sender: &mut UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();
        let gear_api = self.api.gclient_client(&self.suri)?;
        while let Some(Message {
            task_uuid,
            payload,
            tx_hash,
        }) = msg_receiver.recv().await
        {
            let gas_limit_block = gear_api.block_gas_limit()?;
            let gas_limit = gas_limit_block / 100 * 95;

            let remoting = GClientRemoting::new(gear_api.clone());

            let mut proxy_service = HistoricalProxy::new(remoting.clone());

            let (_, receiver_reply) = proxy_service
                .redirect(
                    payload.proof_block.block.slot,
                    payload.encode(),
                    self.vft_mananager_address.into(),
                    route.clone(),
                )
                .with_gas_limit(gas_limit)
                .send_recv(self.historical_proxy_address.into())
                .await
                .map_err(|e| {
                    let error = anyhow::anyhow!("Failed to send message: {e:?}");

                    response_sender
                        .send(Response {
                            task_uuid,
                            status: SendStatus::Failure(error.to_string()),
                        })
                        .unwrap_or_default();
                    error
                })?
                .map_err(|e| {
                    let error = anyhow::anyhow!("Failed to receive message: {e:?}");
                    response_sender
                        .send(Response {
                            task_uuid,
                            status: SendStatus::Failure(error.to_string()),
                        })
                        .unwrap_or_default();
                    error
                })?;
            log::debug!("Received reply: {}", hex::encode(&receiver_reply));

            let reply = SubmitReceipt::decode_reply(&receiver_reply).map_err(|e| {
                let error = anyhow::anyhow!("Failed to decode reply: {}", e);
                response_sender
                    .send(Response {
                        task_uuid,
                        status: SendStatus::Failure(error.to_string()),
                    })
                    .unwrap_or_default();
                error
            })?;

            match reply {
                Ok(()) => {
                    response_sender
                        .send(Response {
                            task_uuid,
                            status: SendStatus::Success,
                        })
                        .unwrap_or_default();
                }
                Err(vft_manager_client::Error::NotSupportedEvent) => {
                    let message = format!("Dropping message for {tx_hash} as it's considered invalid by vft-manager (probably unsupported ERC20 token)");
                    log::warn!("{message}");
                    response_sender
                        .send(Response {
                            task_uuid,
                            status: SendStatus::Failure(message),
                        })
                        .unwrap_or_default();
                }

                Err(e) => {
                    let message = format!("Internal vft-manager error: {e:?}");

                    response_sender
                        .send(Response {
                            task_uuid,
                            status: SendStatus::Failure(message.clone()),
                        })
                        .unwrap_or_default();
                    anyhow::bail!("{message}")
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub payload: EthToVaraEvent,
    pub tx_hash: FixedBytes<32>,
    pub task_uuid: Uuid,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub task_uuid: Uuid,
    pub status: SendStatus,
}

#[derive(Clone, Debug)]
pub enum SendStatus {
    Success,
    Failure(String),
}
