use crate::{
    common::{is_transport_error_recoverable, BASE_RETRY_DELAY},
    message_relayer::common::RelayedMerkleRoot,
};
use ethereum_client::{abi::IMessageQueue::IMessageQueueErrors, EthApi, SubmissionGuard, TxHash};
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
    Failed(Uuid, String),
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
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) {
    match this.eth_api.get_approx_balance().await {
        Ok(fee_payer_balance) => this.metrics.fee_payer_balance.set(fee_payer_balance),
        Err(e) => log::warn!("Failed to update Ethereum fee payer balance metric: {e}"),
    }

    while let Some(request) = requests.recv().await {
        let submission_guard = this.eth_api.reserve_submission().await;
        let mut account_nonce = None;
        loop {
            match process_request(
                &mut this,
                &request,
                &submission_guard,
                &mut account_nonce,
                &responses,
            )
            .await
            {
                Ok(true) => break,
                Ok(false) => return,
                Err(e) => {
                    let delay = BASE_RETRY_DELAY * 6;
                    log::error!(
                        r#"Ethereum message sender failed: "{e:?}". Retrying the same request in {delay:?}""#,
                    );

                    loop {
                        tokio::time::sleep(delay).await;
                        match this.eth_api.reconnect().await {
                            Ok(eth_api) => {
                                this.eth_api = eth_api;
                                log::debug!("EthApi successfully reconnected");
                                break;
                            }
                            Err(e) => {
                                log::error!(
                                    r#"Failed to reconnect to Ethereum: "{e:?}". Retrying in {delay:?}""#
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Processes one request. `Ok(true)` means the request reached a durable outcome,
/// while `Ok(false)` means the response channel was closed and the task should stop.
async fn process_request(
    this: &mut MessageSender,
    request: &Request,
    submission_guard: &SubmissionGuard,
    account_nonce: &mut Option<u64>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<bool> {
    let Request {
        message,
        relayed_root,
        proof,
        tx_uuid,
    } = request;

    let tx_hash = match this
        .eth_api
        .provide_content_message(
            submission_guard,
            relayed_root.block.0,
            proof.num_leaves as u32,
            proof.leaf_index as u32,
            message.nonce_be,
            message.source,
            message.destination,
            message.payload.to_vec(),
            proof.proof.clone(),
            *account_nonce,
        )
        .await
    {
        Ok((tx_hash, nonce)) => {
            *account_nonce = Some(nonce);
            tx_hash
        }
        Err(submission) => {
            if let Some(nonce) = submission.nonce {
                *account_nonce = Some(nonce);
            }

            if matches!(
                &submission.error,
                ethereum_client::Error::MessageQueue(IMessageQueueErrors::MessageAlreadyProcessed(
                    _
                ))
            ) {
                log::info!(
                    "Message with nonce {} already processed, skipping: tx_uuid = {}",
                    hex::encode(message.nonce_be),
                    tx_uuid
                );
                if responses
                    .send(Response::MessageAlreadyProcessed(*tx_uuid))
                    .is_err()
                {
                    log::info!("Response channel closed, exiting");
                    return Ok(false);
                }
                return Ok(true);
            }

            let retry_same_nonce = match &submission.error {
                ethereum_client::Error::ErrorSendingTransaction(error) => {
                    is_same_nonce_retry_error(&error.to_string())
                }
                _ => false,
            };
            let error = anyhow::Error::new(submission.error);
            if retry_same_nonce || is_transport_error_recoverable(&error) {
                return Err(error);
            }

            let error = format!("Failed to provide content message: {error}");
            log::error!("{error}");
            if responses.send(Response::Failed(*tx_uuid, error)).is_err() {
                log::info!("Response channel closed, exiting");
                return Ok(false);
            }
            return Ok(true);
        }
    };

    log::info!(
        "Message with nonce {} relaying started: tx_hash = {tx_hash}",
        hex::encode(message.nonce_be)
    );

    this.metrics.total_submissions.inc();

    if responses
        .send(Response::ProcessingStarted(tx_hash, *tx_uuid))
        .is_err()
    {
        log::info!("Response channel closed, exiting");
        return Ok(false);
    }

    match this.eth_api.get_approx_balance().await {
        Ok(fee_payer_balance) => this.metrics.fee_payer_balance.set(fee_payer_balance),
        Err(e) => log::warn!("Failed to update Ethereum fee payer balance metric: {e}"),
    }

    Ok(true)
}

fn is_same_nonce_retry_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("already known")
        || error.contains("nonce too low")
        || error.contains("replacement transaction underpriced")
        || error.contains("replacement underpriced")
}

#[cfg(test)]
mod tests {
    use super::is_same_nonce_retry_error;

    #[test]
    fn classifies_same_nonce_retry_errors() {
        for error in [
            "already known",
            "nonce too low",
            "replacement transaction underpriced",
            "replacement underpriced",
        ] {
            assert!(is_same_nonce_retry_error(error), "{error}");
        }

        for error in [
            "insufficient funds for gas * price + value",
            "max fee per gas less than block base fee",
            "intrinsic gas too low",
        ] {
            assert!(!is_same_nonce_retry_error(error), "{error}");
        }
    }
}
