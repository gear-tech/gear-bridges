use crate::{common, message_relayer::common::RelayedMerkleRoot};
use ethereum_client::{abi::IMessageQueue::IMessageQueueErrors, EthApi, TxHash};
use gear_rpc_client::dto::{MerkleProof, Message};
use prometheus::{Gauge, IntCounter};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

pub struct Request {
    pub message: Message,
    pub relayed_root: RelayedMerkleRoot,
    pub proof: MerkleProof,
    pub tx_uuid: Uuid,
}

pub enum Response {
    MessageAlreadyProcessed(Uuid),
    ProcessingStarted(TxHash, Uuid),
}

pub struct MessageSenderIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl MessageSenderIo {
    pub fn new(requests: UnboundedSender<Request>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
            responses,
        }
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }

    pub fn send(
        &mut self,
        message: Message,
        relayed_root: RelayedMerkleRoot,
        proof: MerkleProof,
        tx_uuid: Uuid,
    ) -> bool {
        self.requests
            .send(Request {
                message,
                relayed_root,
                proof,
                tx_uuid,
            })
            .is_ok()
    }
}

pub struct MessageSender {
    eth_api: EthApi,

    metrics: Metrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        fee_payer_balance: Gauge = Gauge::new(
            "ethereum_message_sender_fee_payer_balance",
            "Transaction fee payer balance",
        ),

        total_submissions: IntCounter = IntCounter::new(
            "ethereum_message_sender_total_submissions",
            "Total number of merkle root submissions to Ethereum",
        ),
    }
}

impl MessageSender {
    pub fn new(eth_api: EthApi) -> Self {
        Self {
            eth_api,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self) -> MessageSenderIo {
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let (responses_tx, responses_rx) = mpsc::unbounded_channel();

        tokio::task::spawn(task(self, requests_rx, responses_tx));

        MessageSenderIo {
            requests: requests_tx,
            responses: responses_rx,
        }
    }
}

async fn task(
    mut this: MessageSender,
    mut channel_message_data: UnboundedReceiver<Request>,
    channel_tx_data: UnboundedSender<Response>,
) {
    if let Ok(fee_payer_balance) = this.eth_api.get_approx_balance().await {
        this.metrics.fee_payer_balance.set(fee_payer_balance);
    }

    let mut attempts = 0;
    // Holds the request that was popped but not yet successfully processed.
    let mut pending_request: Option<Request> = None;

    loop {
        match task_inner(
            &mut this,
            &mut channel_message_data,
            &channel_tx_data,
            &mut pending_request,
        )
        .await
        {
            Ok(_) => break,
            Err(e) => {
                attempts += 1;
                common::retry_backoff(attempts, "Ethereum message sender", &e).await;

                if common::is_transport_error_recoverable(&e) {
                    // Add simple retry loop for reconnection
                    loop {
                        match this.eth_api.reconnect().await.inspect_err(|e| {
                            log::error!("Failed to reconnect to Ethereum: {e}");
                        }) {
                            Ok(eth_api) => {
                                this.eth_api = eth_api;
                                attempts = 0; // Reset attempts after successful reconnect
                                break;
                            }
                            Err(_) => {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn task_inner(
    this: &mut MessageSender,
    requests: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
    pending_request: &mut Option<Request>,
) -> anyhow::Result<()> {
    loop {
        // If we have a pending request (failed previously), receive it.
        // Otherwise, pull from channel.
        let request = if let Some(req) = pending_request.take() {
            req
        } else {
            match requests.recv().await {
                Some(req) => req,
                None => return Ok(()), // Channel closed
            }
        };

        let tx_hash_res = this
            .eth_api
            .provide_content_message(
                request.relayed_root.block.0,
                request.proof.num_leaves as u32,
                request.proof.leaf_index as u32,
                request.message.nonce_be,
                request.message.source,
                request.message.destination,
                request.message.payload.clone(), // payload might be large
                request.proof.proof.clone(),
            )
            .await;

        let tx_hash = match tx_hash_res {
            Ok(tx_hash) => tx_hash,
            Err(ethereum_client::Error::MessageQueue(
                IMessageQueueErrors::MessageAlreadyProcessed(_),
            )) => {
                log::info!(
                    "Message with nonce {} already processed, skipping: tx_uuid = {}",
                    hex::encode(request.message.nonce_be),
                    request.tx_uuid
                );
                if responses
                    .send(Response::MessageAlreadyProcessed(request.tx_uuid))
                    .is_err()
                {
                    log::info!("Response channel closed, exiting");
                    return Ok(());
                }
                continue;
            }
            Err(e) => {
                // Restore pending request
                *pending_request = Some(request);
                return Err(anyhow::anyhow!("Failed to provide content message: {e}"));
            }
        };

        log::info!(
            "Message with nonce {} relaying started: tx_hash = {tx_hash}",
            hex::encode(request.message.nonce_be)
        );

        this.metrics.total_submissions.inc();

        if responses
            .send(Response::ProcessingStarted(tx_hash, request.tx_uuid))
            .is_err()
        {
            log::info!("Response channel closed, exiting");
            return Ok(());
        }

        let fee_payer_balance = this.eth_api.get_approx_balance().await?;
        this.metrics.fee_payer_balance.set(fee_payer_balance);
    }
}
