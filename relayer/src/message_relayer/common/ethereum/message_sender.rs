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

#[derive(Clone)]
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

        tokio::task::spawn(task(
            self,
            requests_tx.downgrade(),
            requests_rx,
            responses_tx,
        ));

        MessageSenderIo {
            requests: requests_tx,
            responses: responses_rx,
        }
    }
}

async fn task(
    mut this: MessageSender,
    retry_requests: mpsc::WeakUnboundedSender<Request>,
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) {
    match this.eth_api.get_approx_balance().await {
        Ok(fee_payer_balance) => this.metrics.fee_payer_balance.set(fee_payer_balance),
        Err(e) => log::warn!("Failed to update Ethereum fee payer balance metric: {e}"),
    }

    while let Some(request) = requests.recv().await {
        let mut submission_guard = None;
        let mut account_nonce = None;
        loop {
            if submission_guard.is_none() {
                submission_guard = Some(this.eth_api.reserve_submission().await);
            }

            match process_request(
                &mut this,
                &request,
                submission_guard
                    .as_ref()
                    .expect("submission guard was just reserved"),
                &mut account_nonce,
                &responses,
            )
            .await
            {
                Ok(true) => break,
                Ok(false) => return,
                Err(e) => {
                    // No nonce needs protecting when submission did not reach
                    // eth_sendRawTransaction, or when a stale nonce was reset.
                    // Release the guard while waiting so other Ethereum work is
                    // not blocked by a transient contract state indefinitely.
                    if account_nonce.is_none() {
                        drop(submission_guard.take());
                    }

                    let delay = BASE_RETRY_DELAY * 6;
                    log::error!(
                        r#"Ethereum message sender failed: "{e:?}". Retrying the same request in {delay:?}""#,
                    );

                    if account_nonce.is_none() && is_retryable_contract_submission(&e) {
                        // Paused/challenged messages must not head-of-line block
                        // governance messages that are allowed to bypass those states.
                        let request = request.clone();
                        let retry_requests = retry_requests.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(delay).await;
                            let Some(retry_requests) = retry_requests.upgrade() else {
                                log::info!("Message sender request channel closed before retry");
                                return;
                            };
                            if retry_requests.send(request).is_err() {
                                log::info!("Message sender request channel closed before retry");
                            }
                        });
                        break;
                    }

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
                return Ok(report_already_processed(
                    message.nonce_be,
                    *tx_uuid,
                    responses,
                ));
            }

            let nonce_too_low = match &submission.error {
                ethereum_client::Error::ErrorSendingTransaction(error) => {
                    is_nonce_too_low_error(&error.to_string())
                }
                _ => false,
            };
            if nonce_too_low {
                match this.eth_api.is_message_processed(message.nonce_be).await {
                    Ok(true) => {
                        return Ok(report_already_processed(
                            message.nonce_be,
                            *tx_uuid,
                            responses,
                        ));
                    }
                    Ok(false) => {
                        let Some(pinned_nonce) = *account_nonce else {
                            return Err(anyhow::anyhow!(
                                "nonce-too-low submission error did not preserve its account nonce"
                            ));
                        };

                        match this.eth_api.is_account_nonce_consumed(pinned_nonce).await {
                            Ok(true) => {
                                log::warn!(
                                    concat!(
                                        "Pinned Ethereum account nonce {} was consumed, but message ",
                                        "{} is not processed; resetting the account nonce before retry"
                                    ),
                                    pinned_nonce,
                                    hex::encode(message.nonce_be),
                                );
                                *account_nonce = None;
                            }
                            Ok(false) => {
                                log::info!(
                                    concat!(
                                        "Pinned Ethereum account nonce {} is still pending; retaining ",
                                        "it while retrying message {}"
                                    ),
                                    pinned_nonce,
                                    hex::encode(message.nonce_be),
                                );
                            }
                            Err(reconciliation_error) => {
                                return Err(anyhow::Error::new(reconciliation_error).context(
                                    format!(
                                        "failed to check pinned Ethereum account nonce {} while reconciling message {}",
                                        pinned_nonce,
                                        hex::encode(message.nonce_be),
                                    ),
                                ));
                            }
                        }
                    }
                    Err(reconciliation_error) => {
                        return Err(anyhow::Error::new(reconciliation_error).context(format!(
                            "failed to reconcile message {} after Ethereum account nonce was too low",
                            hex::encode(message.nonce_be),
                        )));
                    }
                }
            }

            let retry_same_nonce = match &submission.error {
                ethereum_client::Error::ErrorSendingTransaction(error) => {
                    is_same_nonce_retry_error(&error.to_string())
                }
                _ => false,
            };
            let retry_message_queue = match &submission.error {
                ethereum_client::Error::MessageQueue(error) => {
                    is_retryable_message_queue_error(error)
                }
                _ => false,
            };
            let error = anyhow::Error::new(submission.error);
            if retry_same_nonce || retry_message_queue || is_transport_error_recoverable(&error) {
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

fn report_already_processed(
    message_nonce: [u8; 32],
    tx_uuid: Uuid,
    responses: &UnboundedSender<Response>,
) -> bool {
    log::info!(
        "Message with nonce {} already processed, skipping: tx_uuid = {}",
        hex::encode(message_nonce),
        tx_uuid,
    );
    if responses
        .send(Response::MessageAlreadyProcessed(tx_uuid))
        .is_err()
    {
        log::info!("Response channel closed, exiting");
        return false;
    }
    true
}

fn is_nonce_too_low_error(error: &str) -> bool {
    error.to_ascii_lowercase().contains("nonce too low")
}

fn is_same_nonce_retry_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("already known")
        || is_nonce_too_low_error(&error)
        || error.contains("replacement transaction underpriced")
        || error.contains("replacement underpriced")
}

fn is_retryable_contract_submission(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ethereum_client::Error>(),
        Some(ethereum_client::Error::MessageQueue(error))
            if is_retryable_message_queue_error(error)
    )
}

fn is_retryable_message_queue_error(error: &IMessageQueueErrors) -> bool {
    matches!(
        error,
        IMessageQueueErrors::ChallengeRoot(_)
            | IMessageQueueErrors::EmergencyStop(_)
            | IMessageQueueErrors::EnforcedPause(_)
            | IMessageQueueErrors::MerkleRootDelayNotPassed(_)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        is_nonce_too_low_error, is_retryable_message_queue_error, is_same_nonce_retry_error,
    };
    use ethereum_client::abi::IMessageQueue::{self, IMessageQueueErrors};

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

    #[test]
    fn identifies_nonce_too_low_for_reconciliation() {
        assert!(is_nonce_too_low_error("Nonce Too Low"));
        assert!(!is_nonce_too_low_error("already known"));
    }

    #[test]
    fn classifies_transient_message_queue_errors() {
        for error in [
            IMessageQueueErrors::ChallengeRoot(IMessageQueue::ChallengeRoot {}),
            IMessageQueueErrors::EmergencyStop(IMessageQueue::EmergencyStop {}),
            IMessageQueueErrors::EnforcedPause(IMessageQueue::EnforcedPause {}),
            IMessageQueueErrors::MerkleRootDelayNotPassed(
                IMessageQueue::MerkleRootDelayNotPassed {},
            ),
        ] {
            assert!(is_retryable_message_queue_error(&error), "{error:?}");
        }

        let terminal =
            IMessageQueueErrors::InvalidMerkleProof(IMessageQueue::InvalidMerkleProof {});
        assert!(!is_retryable_message_queue_error(&terminal));
    }
}
