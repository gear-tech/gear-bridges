use crate::{
    common::{is_transport_error_recoverable, BASE_RETRY_DELAY},
    message_relayer::common::RelayedMerkleRoot,
};
use ethereum_client::{
    abi::IMessageQueue::IMessageQueueErrors, transaction_identity, EthApi, PreparedContentMessage,
    SubmissionGuard, TxHash,
};
use gear_rpc_client::dto::{MerkleProof, Message};
use prometheus::{Gauge, IntCounter};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

#[derive(Clone)]
pub struct Request {
    pub message: Message,
    pub relayed_root: RelayedMerkleRoot,
    pub proof: MerkleProof,
    pub tx_uuid: Uuid,
    pub prepared: Option<PreparedContentMessage>,
}

pub enum Response {
    Prepared {
        tx_uuid: Uuid,
        submission: PreparedContentMessage,
        durable: oneshot::Sender<()>,
    },
    ProcessingStarted(TxHash, Uuid),
    Failed(Uuid, String),
    Hold(Uuid, String),
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
        prepared: Option<PreparedContentMessage>,
    ) -> bool {
        self.requests
            .send(Request {
                message,
                relayed_root,
                proof,
                tx_uuid,
                prepared,
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
        let mut prepared = request.prepared.clone();
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
                &mut prepared,
                &responses,
            )
            .await
            {
                Ok(true) => break,
                Ok(false) => return,
                Err(e) => {
                    // Unbroadcast preparations may release the account; saved signatures reserve it until reconciled.
                    if prepared.is_none() {
                        drop(submission_guard.take());
                    }

                    let delay = BASE_RETRY_DELAY * 6;
                    log::error!(
                        r#"Ethereum message sender failed: "{e:?}". Retrying the same request in {delay:?}""#,
                    );

                    if prepared.is_none() && is_retryable_contract_submission(&e) {
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

/// A saved signature may only be reconciled or broadcast verbatim after durable handoff.
async fn process_request(
    this: &mut MessageSender,
    request: &Request,
    submission_guard: &SubmissionGuard,
    prepared: &mut Option<PreparedContentMessage>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<bool> {
    let message = &request.message;
    let root = request.relayed_root;
    let proof = &request.proof;
    let tx_uuid = request.tx_uuid;
    let total_leaves = u32::try_from(proof.num_leaves)?;
    let leaf_index = u32::try_from(proof.leaf_index)?;
    let fresh = prepared.is_none();
    if fresh {
        let signed = match this
            .eth_api
            .prepare_content_message(
                submission_guard,
                root.block.0,
                total_leaves,
                leaf_index,
                message.nonce_be,
                message.source,
                message.destination,
                message.payload.clone(),
                proof.proof.clone(),
            )
            .await
        {
            Ok(signed) => signed,
            Err(error) => {
                if matches!(
                    &error,
                    ethereum_client::Error::MessageQueue(
                        IMessageQueueErrors::MessageAlreadyProcessed(_)
                    )
                ) {
                    return Ok(report_hold(tx_uuid, "HOLD: message nonce already processed without an original signed receipt; another transaction cannot complete this operation".into(), responses));
                }
                let error = anyhow::Error::new(error);
                if is_retryable_contract_submission(&error)
                    || is_transport_error_recoverable(&error)
                {
                    return Err(error);
                }
                return Ok(responses
                    .send(Response::Failed(
                        tx_uuid,
                        format!("Failed to prepare content message before broadcast: {error}"),
                    ))
                    .is_ok());
            }
        };
        *prepared = Some(signed);
    }
    let signed = prepared
        .as_ref()
        .expect("submission was prepared or restored");
    if let Err(error) = this.eth_api.validate_prepared_content_message(
        signed,
        root.block.0,
        total_leaves,
        leaf_index,
        message.nonce_be,
        message.source,
        message.destination,
        message.payload.clone(),
        proof.proof.clone(),
    ) {
        return Ok(report_hold(tx_uuid, format!("HOLD: original outbound signed transaction failed local identity/content validation: {error}"), responses));
    }
    if fresh && !durable_handoff(tx_uuid, signed, responses).await {
        return Ok(false);
    }
    let snapshot = this.eth_api.get_finalized_submission_snapshot(signed.nonce, message.nonce_be).await?
        .ok_or_else(|| anyhow::anyhow!("HOLD: canonical finalized submission snapshot is pending for original outbound transaction {}", signed.hash))?;
    if !snapshot.message_processed && snapshot.pinned_nonce_consumed {
        return Ok(report_hold(tx_uuid, format!("HOLD: original outbound account nonce {} was consumed but message remains unprocessed at finalized block {} ({})", signed.nonce, snapshot.block_number, snapshot.block_hash), responses));
    }
    // Finality may precede a restart: watch the saved hash even when its message
    // is already processed, rather than losing the original successful receipt.
    if let Some(observed) = transaction_identity(this.eth_api.raw_provider(), signed.hash).await? {
        if observed.hash != signed.hash
            || observed.from != signed.sender
            || observed.nonce != signed.nonce
            || observed.to != Some(signed.contract)
        {
            return Ok(report_hold(
                tx_uuid,
                "HOLD: RPC outbound transaction differs from saved signed identity".into(),
                responses,
            ));
        }
    } else if !snapshot.message_processed {
        let observed_hash = this
            .eth_api
            .broadcast_prepared_content_message(submission_guard, signed)
            .await?;
        anyhow::ensure!(
            observed_hash == signed.hash,
            "HOLD: RPC changed original outbound transaction hash"
        );
    }
    log::info!(
        "Message with nonce {} relaying started with original tx_hash = {}",
        hex::encode(message.nonce_be),
        signed.hash
    );
    this.metrics.total_submissions.inc();
    if responses
        .send(Response::ProcessingStarted(signed.hash, tx_uuid))
        .is_err()
    {
        return Ok(false);
    }
    match this.eth_api.get_approx_balance().await {
        Ok(fee_payer_balance) => this.metrics.fee_payer_balance.set(fee_payer_balance),
        Err(e) => log::warn!("Failed to update Ethereum fee payer balance metric: {e}"),
    }
    Ok(true)
}

async fn durable_handoff(
    tx_uuid: Uuid,
    signed: &PreparedContentMessage,
    responses: &UnboundedSender<Response>,
) -> bool {
    let (durable, acknowledged) = oneshot::channel();
    if responses
        .send(Response::Prepared {
            tx_uuid,
            submission: signed.clone(),
            durable,
        })
        .is_err()
    {
        return false;
    }
    acknowledged.await.is_ok()
}

fn report_hold(tx_uuid: Uuid, reason: String, responses: &UnboundedSender<Response>) -> bool {
    log::error!("{reason}");
    responses.send(Response::Hold(tx_uuid, reason)).is_ok()
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
    use super::is_retryable_message_queue_error;
    use ethereum_client::abi::IMessageQueue::{self, IMessageQueueErrors};

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

    #[tokio::test]
    async fn prepared_submission_cannot_cross_handoff_without_durable_acknowledgement() {
        use super::{durable_handoff, Response};
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            eips::Encodable2718,
            network::TxSigner,
            primitives::{Address, Bytes, TxKind, B256, U256},
            signers::local::PrivateKeySigner,
            sol_types::SolCall,
        };
        use ethereum_client::{
            abi::IMessageQueue::{self, VaraMessage},
            PreparedContentMessage,
        };
        use tokio::sync::mpsc;
        use uuid::Uuid;
        let signer = PrivateKeySigner::from_bytes(&B256::from([7; 32])).unwrap();
        let contract = Address::from([2; 20]);
        let input = IMessageQueue::processMessageCall {
            blockNumber: U256::from(43),
            totalLeaves: U256::from(1),
            leafIndex: U256::ZERO,
            message: VaraMessage {
                nonce: U256::from(7),
                source: B256::from([1; 32]),
                destination: Address::from([2; 20]),
                payload: Bytes::from(vec![3, 4]),
            },
            proof: vec![],
        }
        .abi_encode();
        let mut transaction = TxEip1559 {
            chain_id: 560048,
            nonce: 36,
            gas_limit: 100_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(contract),
            input: Bytes::from(input),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut transaction).await.unwrap();
        let raw_transaction =
            TxEnvelope::Eip1559(transaction.into_signed(signature)).encoded_2718();
        let signed = PreparedContentMessage {
            chain_id: 560048,
            contract,
            sender: signer.address(),
            nonce: 36,
            hash: alloy::primitives::keccak256(&raw_transaction),
            raw_transaction,
        };
        for acknowledge in [false, true] {
            let (responses, mut received) = mpsc::unbounded_channel();
            let uuid = Uuid::new_v4();
            let saved = signed.clone();
            let waiting =
                tokio::spawn(async move { durable_handoff(uuid, &saved, &responses).await });
            let Response::Prepared {
                tx_uuid,
                submission,
                durable,
            } = received.recv().await.unwrap()
            else {
                panic!("expected original signed handoff");
            };
            assert_eq!(tx_uuid, uuid);
            assert!(submission == signed);
            assert!(!waiting.is_finished());
            if acknowledge {
                durable.send(()).unwrap();
            } else {
                drop(durable);
            }
            assert_eq!(waiting.await.unwrap(), acknowledge);
        }
    }
}
