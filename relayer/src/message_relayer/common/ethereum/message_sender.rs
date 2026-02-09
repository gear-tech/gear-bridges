use crate::{common::BASE_RETRY_DELAY, message_relayer::common::RelayedMerkleRoot};
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

    loop {
        let Err(e) = task_inner(&mut this, &mut channel_message_data, &channel_tx_data).await
        else {
            break;
        };

        let delay = BASE_RETRY_DELAY * 6;
        log::error!(r#"Ethereum message sender failed: "{e:?}". Retrying in {delay:?}"#,);

        tokio::time::sleep(delay).await;
        match this.eth_api.reconnect().await {
            Ok(eth_api) => this.eth_api = eth_api,
            Err(e) => {
                log::error!(r#"Failed to reconnect to Ethereum: "{e:?}""#);
                break;
            }
        }
    }
}

async fn task_inner(
    this: &mut MessageSender,
    requests: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<()> {
    while let Some(request) = requests.recv().await {
        let Request {
            message,
            relayed_root,
            proof,
            tx_uuid,
        } = request;

        let tx_hash = match this
            .eth_api
            .provide_content_message(
                relayed_root.block.0,
                proof.num_leaves as u32,
                proof.leaf_index as u32,
                message.nonce_be,
                message.source,
                message.destination,
                message.payload.to_vec(),
                proof.proof,
            )
            .await
        {
            Ok(tx_hash) => tx_hash,
            Err(ethereum_client::Error::MessageQueue(
                IMessageQueueErrors::MessageAlreadyProcessed(_),
            )) => {
                log::info!(
                    "Message with nonce {} already processed, skipping: tx_uuid = {}",
                    hex::encode(message.nonce_be),
                    tx_uuid
                );
                if responses
                    .send(Response::MessageAlreadyProcessed(tx_uuid))
                    .is_err()
                {
                    log::info!("Response channel closed, exiting");
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(anyhow::anyhow!("Failed to provide content message: {e}")),
        };

        log::info!(
            "Message with nonce {} relaying started: tx_hash = {tx_hash}",
            hex::encode(message.nonce_be)
        );

        this.metrics.total_submissions.inc();

        if responses
            .send(Response::ProcessingStarted(tx_hash, tx_uuid))
            .is_err()
        {
            log::info!("Response channel closed, exiting");
            return Ok(());
        }

        let fee_payer_balance = this.eth_api.get_approx_balance().await?;
        this.metrics.fee_payer_balance.set(fee_payer_balance);
    }

    Ok(())
}
