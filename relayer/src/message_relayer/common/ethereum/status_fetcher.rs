use crate::{common::BASE_RETRY_DELAY, message_relayer::common::message_hash};
use alloy::{
    primitives::{Address, B256, U256},
    sol_types::{SolCall, SolEvent},
};
use ethereum_client::{
    abi::{IERC20Manager, IMessageQueue},
    EthApi, FinalizedSubmissionSnapshot, FinalizedTransactionReceipt, PreparedContentMessage,
    TxHash,
};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use gear_rpc_client::dto::Message;
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    IntCounter, IntGauge,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

// Discover the token destination through the original queue's governance, at
// the receipt inclusion hash; payload length alone never classifies an app.
alloy::sol! { #[sol(rpc)] interface QueueGovernance { function erc20Manager() external view returns (address); } }

pub struct StatusFetcher {
    eth_api: EthApi,
    confirmations: u64,
    metrics: Metrics,
}

#[derive(Clone)]
struct TrackedRequest {
    tx_uuid: Uuid,
    tx_hash: TxHash,
    message: Message,
    submission: Option<PreparedContentMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionEvidence {
    pub receipt: alloy::rpc::types::TransactionReceipt,
    pub finalized_block_number: u64,
    pub finalized_block_hash: B256,
    pub token_delivery: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizedFailureEvidence {
    pub receipt: alloy::rpc::types::TransactionReceipt,
    pub included_block_number: u64,
    pub included_block_hash: B256,
    pub finalized_block_number: u64,
    pub finalized_block_hash: B256,
}

#[derive(Clone, Debug)]
pub enum SubmissionOutcome {
    Completed(CompletionEvidence),
    FinalizedRevert(FinalizedFailureEvidence),
    NeedsReconciliation {
        diagnostic: String,
        nonce_disposition: Option<FinalizedSubmissionSnapshot>,
    },
}

const TX_VISIBILITY_RECHECK_DELAY: Duration = Duration::from_secs(15);
type TxWatch = BoxFuture<'static, (TrackedRequest, SubmissionOutcome)>;
pub struct StatusFetcherIo {
    requests: UnboundedSender<TrackedRequest>,
    responses: UnboundedReceiver<(Uuid, SubmissionOutcome)>,
}
impl StatusFetcherIo {
    pub fn send_request(
        &self,
        tx_uuid: Uuid,
        tx_hash: TxHash,
        message: Message,
        submission: Option<PreparedContentMessage>,
    ) -> bool {
        self.requests
            .send(TrackedRequest {
                tx_uuid,
                tx_hash,
                message,
                submission,
            })
            .is_ok()
    }
    pub async fn recv_message(&mut self) -> Option<(Uuid, SubmissionOutcome)> {
        self.responses.recv().await
    }
}
impl MeteredService for StatusFetcher {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}
impl_metered_service! {
    struct Metrics {
        pending_tx_count: IntGauge = IntGauge::new(
            "ethereum_message_sender_pending_tx_count",
            "Amount of txs pending finalization on ethereum",
        ),
        total_failed_txs: IntCounter = IntCounter::new(
            "ethereum_message_sender_total_failed_txs",
            "Total amount of txs sent to ethereum and failed",
        ),

        total_gas_used: GenericCounter<AtomicU64> = GenericCounter::new(
            "ethereum_message_sender_total_gas_used",
            "Total gas used by ethereum message sender",
        ),
        min_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "ethereum_message_sender_min_gas_used",
            "Minimum gas used by ethereum message sender",
        ),
        max_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "ethereum_message_sender_max_gas_used",
            "Maximum gas used by ethereum message sender",
        ),
        last_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "ethereum_message_sender_last_gas_used",
            "Last gas used by ethereum message sender",
        ),
    }
}

impl StatusFetcher {
    pub fn new(eth_api: EthApi, confirmations: u64) -> Self {
        Self {
            eth_api,
            confirmations,
            metrics: Metrics::new(),
        }
    }
    pub fn spawn(self) -> StatusFetcherIo {
        let (requests, receiver) = mpsc::unbounded_channel();
        let (responses, results) = mpsc::unbounded_channel();
        tokio::spawn(task(self, receiver, responses));
        StatusFetcherIo {
            requests,
            responses: results,
        }
    }
}
async fn task(
    this: StatusFetcher,
    mut requests: UnboundedReceiver<TrackedRequest>,
    responses: UnboundedSender<(Uuid, SubmissionOutcome)>,
) {
    let mut watches = FuturesUnordered::new();
    let mut registered = HashSet::new();
    let mut failed_originals = HashSet::new();
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { return };
                // Restored originals and duplicate registration share one read-only watch.
                if !registered.insert((request.tx_uuid, request.tx_hash)) { continue }
                this.metrics.pending_tx_count.inc();
                watches.push(watch_tx(this.eth_api.clone(), request, this.confirmations));
            }
            Some((request, outcome)) = watches.next(), if !watches.is_empty() => {
                this.metrics.pending_tx_count.dec();
                match &outcome {
                    SubmissionOutcome::Completed(evidence) => {
                        this.metrics.total_gas_used.inc_by(evidence.receipt.gas_used);
                        let gas = evidence.receipt.gas_used;
                        this.metrics.last_gas_used.set(gas);
                        let min = this.metrics.min_gas_used.get();
                        if min == 0 || gas < min { this.metrics.min_gas_used.set(gas); }
                        if gas > this.metrics.max_gas_used.get() { this.metrics.max_gas_used.set(gas); }
                    }
                    SubmissionOutcome::FinalizedRevert(_) => {
                        if failed_originals.insert(request.tx_hash) { this.metrics.total_failed_txs.inc(); }
                    }
                    SubmissionOutcome::NeedsReconciliation { .. } => {
                        // Holds are operations with unresolved effects, never failed transactions.
                        registered.remove(&(request.tx_uuid, request.tx_hash));
                    }
                };
                if responses.send((request.tx_uuid, outcome)).is_err() { return }
            }
        }
    }
}

fn original_call(
    message: &Message,
    signed: &PreparedContentMessage,
) -> anyhow::Result<IMessageQueue::processMessageCall> {
    use alloy::{
        consensus::{Transaction, TxEnvelope},
        eips::Decodable2718,
    };
    ethereum_client::prepared_content_message_identity(signed)?;
    let mut raw = signed.raw_transaction.as_slice();
    let envelope = TxEnvelope::decode_2718(&mut raw)?;
    let call = IMessageQueue::processMessageCall::abi_decode_validate(envelope.input())?;
    anyhow::ensure!(
        envelope.value() == U256::ZERO
            && envelope.input().as_ref() == call.abi_encode().as_slice()
            && call.message.nonce == U256::from_be_bytes(message.nonce_be)
            && call.message.source == B256::from(message.source)
            && call.message.destination == Address::from(message.destination)
            && call.message.payload.as_ref() == message.payload.as_slice(),
        "HOLD: signed original message differs from source identity"
    );
    Ok(call)
}

pub fn validate_original_submission(
    message: &Message,
    signed: &PreparedContentMessage,
) -> anyhow::Result<()> {
    original_call(message, signed).map(|_| ())
}

pub fn validate_completion(
    message: &Message,
    signed: &PreparedContentMessage,
    evidence: &CompletionEvidence,
) -> anyhow::Result<()> {
    let call = original_call(message, signed)?;
    let receipt = &evidence.receipt;
    anyhow::ensure!(
        receipt.transaction_hash == signed.hash
            && receipt.from == signed.sender
            && receipt.to == Some(signed.contract)
            && receipt.status(),
        "HOLD: original receipt is reverted or differs from signed identity"
    );
    let number = receipt
        .block_number
        .ok_or_else(|| anyhow::anyhow!("HOLD: receipt has no inclusion height"))?;
    anyhow::ensure!(
        receipt.block_hash.is_some_and(|hash| hash != B256::ZERO)
            && evidence.finalized_block_hash != B256::ZERO
            && evidence.finalized_block_number >= number,
        "HOLD: receipt has no finalized inclusion evidence"
    );
    let mut processed = 0;
    let mut bridged = 0;
    for log in receipt.inner.logs() {
        anyhow::ensure!(
            !log.removed
                && log.transaction_hash == Some(signed.hash)
                && log.block_hash == receipt.block_hash
                && log.block_number == receipt.block_number,
            "HOLD: receipt contains a removed or differently included log"
        );
        if log.address() == signed.contract
            && log.topic0() == Some(&IMessageQueue::MessageProcessed::SIGNATURE_HASH)
        {
            let event = IMessageQueue::MessageProcessed::decode_raw_log_validate(
                log.topics(),
                &log.data().data,
            )?;
            anyhow::ensure!(
                event.blockNumber == call.blockNumber
                    && event.messageHash == B256::from(message_hash(message))
                    && event.messageNonce == U256::from_be_bytes(message.nonce_be)
                    && event.messageDestination == Address::from(message.destination),
                "HOLD: queue processing event conflicts with original source message"
            );
            processed += 1;
        }
        if evidence.token_delivery
            && log.address() == Address::from(message.destination)
            && log.topic0() == Some(&IERC20Manager::Bridged::SIGNATURE_HASH)
        {
            let event =
                IERC20Manager::Bridged::decode_raw_log_validate(log.topics(), &log.data().data)?;
            anyhow::ensure!(
                message.payload.len() == 104,
                "HOLD: malformed token payload"
            );
            anyhow::ensure!(
                event.from.as_slice() == &message.payload[..32]
                    && event.to.as_slice() == &message.payload[32..52]
                    && event.token.as_slice() == &message.payload[52..72]
                    && event.amount == U256::from_be_slice(&message.payload[72..]),
                "HOLD: same-receipt token effect conflicts with original payload"
            );
            bridged += 1;
        }
    }
    anyhow::ensure!(processed == 1 && (!evidence.token_delivery || bridged == 1),
        "HOLD: original receipt lacks exactly one matching queue/token effect (queue={processed}, token={bridged})");
    Ok(())
}

pub fn validate_failure(
    message: &Message,
    signed: &PreparedContentMessage,
    evidence: &FinalizedFailureEvidence,
) -> anyhow::Result<()> {
    validate_original_submission(message, signed)?;
    let receipt = &evidence.receipt;
    anyhow::ensure!(
        receipt.transaction_hash == signed.hash
            && receipt.from == signed.sender
            && receipt.to == Some(signed.contract)
            && !receipt.status()
            && receipt.inner.logs().is_empty()
            && receipt.transaction_index.is_some()
            && receipt.block_number == Some(evidence.included_block_number)
            && receipt.block_hash == Some(evidence.included_block_hash)
            && evidence.included_block_hash != B256::ZERO
            && evidence.finalized_block_hash != B256::ZERO
            && evidence.finalized_block_number >= evidence.included_block_number
            && (evidence.finalized_block_number != evidence.included_block_number
                || evidence.finalized_block_hash == evidence.included_block_hash),
        "HOLD: reverted original receipt differs from signed identity or finalized inclusion"
    );
    Ok(())
}

pub async fn failure_evidence(
    api: &EthApi,
    message: &Message,
    signed: &PreparedContentMessage,
    finalized: FinalizedTransactionReceipt,
) -> anyhow::Result<FinalizedFailureEvidence> {
    let evidence = FinalizedFailureEvidence {
        receipt: finalized.receipt,
        included_block_number: finalized.included_block_number,
        included_block_hash: finalized.included_block_hash,
        finalized_block_number: finalized.finalized_block_number,
        finalized_block_hash: finalized.finalized_block_hash,
    };
    validate_failure(message, signed, &evidence)?;
    let mut view = api.verified_finalized_view().await?;
    view.verify_blocks(
        api.raw_provider(),
        &BTreeMap::from([
            (evidence.included_block_number, evidence.included_block_hash),
            (
                evidence.finalized_block_number,
                evidence.finalized_block_hash,
            ),
        ]),
    )
    .await?;
    Ok(evidence)
}

pub async fn completion_evidence(
    api: &EthApi,
    message: &Message,
    signed: &PreparedContentMessage,
    finalized: FinalizedTransactionReceipt,
) -> anyhow::Result<CompletionEvidence> {
    let pin = alloy::rpc::types::BlockId::hash_canonical(finalized.included_block_hash);
    let queue = IMessageQueue::new(signed.contract, api.raw_provider());
    let governance = queue.governanceAdmin().block(pin).call().await?;
    let manager = QueueGovernance::new(governance, api.raw_provider())
        .erc20Manager()
        .block(pin)
        .call()
        .await?;
    let token_delivery =
        if manager != Address::ZERO && manager == Address::from(message.destination) {
            IERC20Manager::new(manager, api.raw_provider())
                .isVftManager(B256::from(message.source))
                .block(pin)
                .call()
                .await?
        } else {
            false
        };
    let evidence = CompletionEvidence {
        receipt: finalized.receipt,
        finalized_block_number: finalized.finalized_block_number,
        finalized_block_hash: finalized.finalized_block_hash,
        token_delivery,
    };
    validate_completion(message, signed, &evidence)?;
    Ok(evidence)
}

fn finalized_receipt_has_required_confirmations(included: u64, head: u64, required: u64) -> bool {
    included
        .checked_add(required.saturating_sub(1))
        .is_some_and(|confirmed| head >= confirmed)
}
fn watch_tx(mut api: EthApi, request: TrackedRequest, confirmations: u64) -> TxWatch {
    Box::pin(async move {
        let hold = |diagnostic: String| SubmissionOutcome::NeedsReconciliation {
            diagnostic,
            nonce_disposition: None,
        };
        let Some(signed) = request.submission.as_ref() else {
            return (
                request,
                hold("HOLD: original signed transaction bytes/nonce are missing".into()),
            );
        };
        let identity = original_call(&request.message, signed).and_then(|call| {
            anyhow::ensure!(
                signed.hash == request.tx_hash,
                "HOLD: original hash differs from watcher identity"
            );
            api.validate_prepared_content_message(
                signed,
                call.blockNumber.try_into()?,
                call.totalLeaves.try_into()?,
                call.leafIndex.try_into()?,
                request.message.nonce_be,
                request.message.source,
                request.message.destination,
                request.message.payload.clone(),
                call.proof.into_iter().map(|hash| hash.0).collect(),
            )
        });
        if let Err(error) = identity {
            return (request, hold(error.to_string()));
        }
        loop {
            let signed = request
                .submission
                .as_ref()
                .expect("original validated above");
            match api.get_finalized_receipt(request.tx_hash).await {
                Ok(Some(receipt)) => {
                    if finalized_receipt_has_required_confirmations(
                        receipt.included_block_number,
                        receipt.finalized_block_number,
                        confirmations,
                    ) {
                        let result = if receipt.receipt.status() {
                            completion_evidence(&api, &request.message, signed, receipt)
                                .await
                                .map(SubmissionOutcome::Completed)
                        } else {
                            failure_evidence(&api, &request.message, signed, receipt)
                                .await
                                .map(SubmissionOutcome::FinalizedRevert)
                        };
                        match result {
                            Ok(outcome) => return (request, outcome),
                            Err(error) if evidence_error_is_retryable(&error) => {
                                log::warn!("Original receipt evidence remains pending: {error}");
                                match api.reconnect().await {
                                    Ok(next) => api = next,
                                    Err(error) => log::warn!("Reconnect failed: {error}"),
                                }
                            }
                            Err(error) => {
                                let diagnostic =
                                    format!("HOLD: original receipt {}: {error}", request.tx_hash);
                                return (request, hold(diagnostic));
                            }
                        }
                    }
                }
                Ok(None) => {
                    // Another caller's processed bit never proves this original's effects.
                    match api
                        .get_finalized_submission_snapshot(signed.nonce, request.message.nonce_be)
                        .await
                    {
                        Ok(Some(snapshot)) if snapshot.pinned_nonce_consumed => {
                            match api
                                .verify_finalized_submission_snapshot(
                                    signed.nonce,
                                    request.message.nonce_be,
                                    &snapshot,
                                )
                                .await
                            {
                                Ok(()) => {
                                    // The nonce read can race original inclusion; qualify its receipt again first.
                                    match api.get_finalized_receipt(request.tx_hash).await {
                                        Ok(None) => return (request, SubmissionOutcome::NeedsReconciliation {
                                            diagnostic: "HOLD: original receipt absent with authenticated finalized consumed nonce".into(),
                                            nonce_disposition: Some(snapshot),
                                        }),
                                        Ok(Some(_)) => continue,
                                        Err(error) => log::warn!("Original receipt recheck remains pending: {error}"),
                                    }
                                }
                                Err(error) => log::warn!(
                                    "Original nonce disposition remains pending: {error}"
                                ),
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            log::warn!("Original nonce disposition remains pending: {error}")
                        }
                    }
                }
                Err(error) => {
                    log::warn!("Canonical original receipt remains pending: {error}");
                    match api.reconnect().await {
                        Ok(next) => api = next,
                        Err(error) => log::warn!("Reconnect failed: {error}"),
                    }
                }
            }
            tokio::time::sleep(TX_VISIBILITY_RECHECK_DELAY.max(BASE_RETRY_DELAY)).await;
        }
    })
}

fn evidence_error_is_retryable(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ethereum_client::Error>(),
        Some(
            ethereum_client::Error::FinalizedAncestryPending
                | ethereum_client::Error::FinalizedAncestryConflict
                | ethereum_client::Error::ErrorFetchingBlock
        )
    ) || matches!(error.downcast_ref::<ethereum_client::Error>(),
            Some(ethereum_client::Error::ErrorInHTTPTransport(error))
                if crate::common::is_rpc_transport_error_recoverable(error))
        || matches!(error.downcast_ref::<alloy::contract::Error>(),
            Some(alloy::contract::Error::TransportError(error))
                if crate::common::is_rpc_transport_error_recoverable(error))
        || matches!(error.downcast_ref::<alloy::transports::TransportError>(),
            Some(error) if crate::common::is_rpc_transport_error_recoverable(error))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloy::providers::Provider;

    use alloy::{
        primitives::{Address, Bytes, B256},
        rpc::types::Block,
    };
    use futures::SinkExt;
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    };
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    pub(crate) async fn signed_message(message: &Message) -> PreparedContentMessage {
        signed_message_with_nonce(message, 1).await
    }
    async fn signed_message_with_nonce(message: &Message, nonce: u64) -> PreparedContentMessage {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            eips::Encodable2718,
            network::TxSigner,
            primitives::{Bytes, TxKind},
            signers::local::PrivateKeySigner,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::from([7; 32])).unwrap();
        let contract = Address::from([2; 20]);
        let call = IMessageQueue::processMessageCall {
            blockNumber: U256::from(43),
            totalLeaves: U256::from(1),
            leafIndex: U256::ZERO,
            message: IMessageQueue::VaraMessage {
                nonce: U256::from_be_bytes(message.nonce_be),
                source: B256::from(message.source),
                destination: Address::from(message.destination),
                payload: Bytes::copy_from_slice(&message.payload),
            },
            proof: vec![],
        };
        let mut tx = TxEip1559 {
            chain_id: 560048,
            nonce,
            gas_limit: 1000000,
            max_fee_per_gas: 2000000000,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(contract),
            input: Bytes::from(call.abi_encode()),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await.unwrap();
        let raw_transaction = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        PreparedContentMessage {
            chain_id: 560048,
            contract,
            sender: signer.address(),
            nonce,
            hash: alloy::primitives::keccak256(&raw_transaction),
            raw_transaction,
        }
    }
    pub(crate) fn evidence(
        message: &Message,
        signed: &PreparedContentMessage,
        token_delivery: bool,
    ) -> CompletionEvidence {
        let processed = IMessageQueue::MessageProcessed {
            blockNumber: U256::from(43),
            messageHash: B256::from(message_hash(message)),
            messageNonce: U256::from_be_bytes(message.nonce_be),
            messageDestination: Address::from(message.destination),
        };
        let mut logs = vec![log_json(
            signed.contract,
            signed.hash,
            processed.encode_log_data(),
        )];
        if token_delivery {
            let bridged = IERC20Manager::Bridged {
                from: B256::from_slice(&message.payload[..32]),
                to: Address::from_slice(&message.payload[32..52]),
                token: Address::from_slice(&message.payload[52..72]),
                amount: U256::from_be_slice(&message.payload[72..]),
            };
            logs.push(log_json(
                Address::from(message.destination),
                signed.hash,
                bridged.encode_log_data(),
            ));
        }
        let receipt = serde_json::from_value(json!({
            "type": "0x2", "status": "0x1", "cumulativeGasUsed": "0x5208", "logs": logs,
            "logsBloom": format!("0x{}", "00".repeat(256)), "transactionHash": signed.hash, "transactionIndex": "0x0",
            "blockHash": B256::from([3; 32]), "blockNumber": "0x64", "gasUsed": "0x5208", "effectiveGasPrice": "0x1",
            "from": signed.sender, "to": signed.contract, "contractAddress": null,
        })).unwrap();
        CompletionEvidence {
            receipt,
            finalized_block_number: 107,
            finalized_block_hash: B256::from([4; 32]),
            token_delivery,
        }
    }
    fn log_json(address: Address, hash: TxHash, data: alloy::primitives::LogData) -> Value {
        json!({ "address": address, "topics": data.topics(), "data": data.data, "transactionHash": hash,
            "blockHash": B256::from([3; 32]), "blockNumber": "0x64", "transactionIndex": "0x0", "logIndex": "0x0", "removed": false })
    }
    #[tokio::test]
    async fn original_receipt_requires_exact_queue_and_same_receipt_token_effect() {
        let message = Message {
            nonce_be: [7; 32],
            source: [8; 32],
            destination: [5; 20],
            payload: vec![6; 104],
        };
        let signed = signed_message(&message).await;
        let complete = evidence(&message, &signed, true);
        validate_completion(&message, &signed, &complete).unwrap();
        for case in [
            "missing",
            "wrong-amount",
            "wrong-sender",
            "wrong-receiver",
            "wrong-token",
            "competing",
            "reverted",
            "wrong-source",
            "missing-queue",
            "removed",
            "duplicate-queue",
            "wrong-root",
            "nonfinal",
            "wrong-origin",
        ] {
            let mut value = serde_json::to_value(&complete).unwrap();
            match case {
                "missing" => {
                    value["receipt"]["logs"].as_array_mut().unwrap().pop();
                }
                "wrong-amount" => {
                    value["receipt"]["logs"][1]["data"] = json!(format!("0x{:064x}", 1))
                }
                "wrong-sender" => value["receipt"]["logs"][1]["topics"][1] = json!(B256::ZERO),
                "wrong-receiver" => value["receipt"]["logs"][1]["topics"][2] = json!(B256::ZERO),
                "wrong-token" => value["receipt"]["logs"][1]["topics"][3] = json!(B256::ZERO),
                "competing" => value["receipt"]["transactionHash"] = json!(B256::ZERO),
                "reverted" => value["receipt"]["status"] = json!("0x0"),
                "wrong-source" => {
                    value["receipt"]["logs"][0]["data"] = json!(format!("0x{}", "00".repeat(128)))
                }
                "missing-queue" => {
                    value["receipt"]["logs"].as_array_mut().unwrap().remove(0);
                }
                "removed" => value["receipt"]["logs"][0]["removed"] = json!(true),
                "duplicate-queue" => {
                    let duplicate = value["receipt"]["logs"][0].clone();
                    value["receipt"]["logs"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                "wrong-root" => {
                    let event = IMessageQueue::MessageProcessed {
                        blockNumber: U256::from(44),
                        messageHash: B256::from(message_hash(&message)),
                        messageNonce: U256::from_be_bytes(message.nonce_be),
                        messageDestination: Address::from(message.destination),
                    };
                    value["receipt"]["logs"][0] =
                        log_json(signed.contract, signed.hash, event.encode_log_data());
                }
                "nonfinal" => value["finalized_block_number"] = json!(99),
                "wrong-origin" => value["receipt"]["from"] = json!(Address::ZERO),
                _ => unreachable!(),
            }
            let wrong: CompletionEvidence = serde_json::from_value(value).unwrap();
            assert!(
                validate_completion(&message, &signed, &wrong).is_err(),
                "{case}"
            );
        }
        let generic = Message {
            payload: vec![9; 104],
            ..message
        };
        let signed = signed_message(&generic).await;
        validate_completion(&generic, &signed, &evidence(&generic, &signed, false)).unwrap();
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RpcFault {
        None,
        Receipt,
        SilentReceipt,
        SilentEvidence,
        DelayedReceipt,
        MissingFinality,
        InvalidFinality,
        ConflictingFinality,
    }

    struct WatchRpcState {
        request: TrackedRequest,
        chain: Vec<Block>,
        fork: Block,
        original_visible: bool,
        processed: AtomicBool,
        account_nonce: AtomicU64,
        head: AtomicU64,
        finalized_height: AtomicU64,
        receipt: Mutex<Value>,
        manager: Mutex<Address>,
        fault: Mutex<RpcFault>,
        calls: Mutex<Vec<Value>>,
    }

    impl WatchRpcState {
        fn receipt(&self, hash: TxHash) -> Value {
            json!({
                "type": "0x2", "status": "0x1", "cumulativeGasUsed": "0x5208",
                "logs": [], "logsBloom": format!("0x{}", "00".repeat(256)),
                "transactionHash": hash, "transactionIndex": "0x0",
                "blockHash": self.chain[100].header.hash, "blockNumber": "0x64",
                "gasUsed": "0x5208", "effectiveGasPrice": "0x1",
                "from": self.request.submission.as_ref().unwrap().sender, "to": Address::from([2; 20]),
                "contractAddress": null,
            })
        }

        fn response(&self, request: &Value) -> Value {
            self.calls.lock().unwrap().push(request.clone());
            let fault = *self.fault.lock().unwrap();
            let params = &request["params"];
            let result = match request["method"].as_str().unwrap() {
                "eth_chainId" => Ok(json!("0x88bb0")),
                "eth_blockNumber" => Ok(json!(format!("0x{:x}", self.head.load(Ordering::SeqCst)))),
                "eth_getTransactionReceipt" if fault == RpcFault::Receipt => {
                    Err("receipt RPC unavailable")
                }
                "eth_getTransactionReceipt" if fault == RpcFault::DelayedReceipt => {
                    *self.fault.lock().unwrap() = RpcFault::None;
                    Ok(Value::Null)
                }
                "eth_getTransactionReceipt" => {
                    assert_eq!(params[0], json!(self.request.tx_hash));
                    Ok(self.receipt.lock().unwrap().clone())
                }
                "eth_getTransactionCount" => {
                    assert_eq!(
                        params[0],
                        json!(self.request.submission.as_ref().unwrap().sender)
                    );
                    assert_eq!(params[1]["requireCanonical"], json!(true));
                    assert!(self
                        .chain
                        .iter()
                        .any(|block| json!(block.header.hash) == params[1]["blockHash"]));
                    Ok(json!(format!(
                        "0x{:x}",
                        self.account_nonce.load(Ordering::SeqCst)
                    )))
                }
                "eth_getTransactionByHash" => Ok(if self.original_visible {
                    json!({
                        "hash": self.request.tx_hash, "nonce": "0x1", "type": "0x2",
                        "from": Address::from([1; 20]), "to": Address::from([2; 20]),
                        "value": "0x0", "input": "0x", "gas": "0x100000",
                        "gasPrice": "0x1", "maxFeePerGas": "0x2", "maxPriorityFeePerGas": "0x1",
                        "chainId": "0x88bb0", "accessList": [], "r": "0x1", "s": "0x2",
                        "v": "0x0", "yParity": "0x0", "blockHash": null,
                        "blockNumber": null, "transactionIndex": null,
                    })
                } else {
                    Value::Null
                }),
                "eth_getBlockByNumber" => {
                    if params[0] == "finalized" {
                        match fault {
                            RpcFault::MissingFinality => Ok(Value::Null),
                            RpcFault::InvalidFinality => {
                                let mut block = self.chain[100].clone();
                                block.header.hash = B256::from([3; 32]);
                                Ok(serde_json::to_value(block).unwrap())
                            }
                            RpcFault::ConflictingFinality => {
                                Ok(serde_json::to_value(&self.fork).unwrap())
                            }
                            _ => Ok(serde_json::to_value(
                                &self.chain[self.finalized_height.load(Ordering::SeqCst) as usize],
                            )
                            .unwrap()),
                        }
                    } else {
                        let number = if params[0] == "latest" {
                            self.head.load(Ordering::SeqCst)
                        } else {
                            u64::from_str_radix(
                                params[0].as_str().unwrap().trim_start_matches("0x"),
                                16,
                            )
                            .unwrap()
                        };
                        Ok(serde_json::to_value(&self.chain[number as usize]).unwrap())
                    }
                }
                "eth_getBlockByHash" => Ok(self
                    .chain
                    .iter()
                    .find(|block| json!(block.header.hash) == params[0])
                    .map(|block| serde_json::to_value(block).unwrap())
                    .unwrap_or(Value::Null)),
                "eth_call" => {
                    let data = params[0]
                        .get("input")
                        .or_else(|| params[0].get("data"))
                        .unwrap()
                        .as_str()
                        .unwrap();
                    if data
                        == format!(
                            "0x{}",
                            hex::encode(IMessageQueue::governanceAdminCall::SELECTOR)
                        )
                    {
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", U256::from_be_slice(Address::from([6; 20]).as_slice()))});
                    }
                    if data
                        == format!(
                            "0x{}",
                            hex::encode(QueueGovernance::erc20ManagerCall::SELECTOR)
                        )
                    {
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", U256::from_be_slice(self.manager.lock().unwrap().as_slice()))});
                    }
                    if data.starts_with(&format!(
                        "0x{}",
                        hex::encode(IERC20Manager::isVftManagerCall::SELECTOR)
                    )) {
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", 1)});
                    }
                    assert_eq!(params[0]["to"], json!(Address::from([2; 20])));
                    assert_eq!(params[1]["requireCanonical"], json!(true));
                    assert!(self
                        .chain
                        .iter()
                        .any(|block| json!(block.header.hash) == params[1]["blockHash"]));
                    assert!(params[0]
                        .get("input")
                        .or_else(|| params[0].get("data"))
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .ends_with(&hex::encode(self.request.message.nonce_be)));
                    Ok(json!(format!(
                        "0x{:064x}",
                        u8::from(self.processed.load(Ordering::SeqCst))
                    )))
                }
                "eth_subscribe" => Ok(json!("0x1")),
                "eth_unsubscribe" => Ok(json!(true)),
                _ => Err("unexpected RPC method"),
            };
            match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
                Err(message) => json!({"jsonrpc": "2.0", "id": request["id"],
                    "error": {"code": -32000, "message": message}}),
            }
        }
    }
    // Each fixture invents a network; keep concurrent fixtures within the shared ancestry cache bound.
    static WATCH_RPC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct WatchRpc {
        api: EthApi,
        state: Arc<WatchRpcState>,
        server: tokio::task::JoinHandle<()>,
        _network: tokio::sync::MutexGuard<'static, ()>,
    }

    impl Drop for WatchRpc {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn watch_rpc(original_visible: bool) -> anyhow::Result<WatchRpc> {
        let network = WATCH_RPC_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}", listener.local_addr()?);
        let message = Message {
            nonce_be: [7; 32],
            source: [8; 32],
            destination: [5; 20],
            payload: vec![6; 104],
        };
        let signed = signed_message(&message).await;
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: signed.hash,
            message,
            submission: Some(signed),
        };
        let mut parent = B256::ZERO;
        let chain: Vec<Block> = (0..=107)
            .map(|number| {
                let mut block: Block = Block::default();
                block.header.inner.number = number;
                block.header.inner.parent_hash = parent;
                block.header.inner.extra_data = Bytes::copy_from_slice(request.tx_uuid.as_bytes());
                block.header.hash = block.header.inner.hash_slow();
                parent = block.header.hash;
                block
            })
            .collect();
        let mut fork = chain[100].clone();
        fork.header.inner.extra_data = Bytes::from(vec![4]);
        fork.header.hash = fork.header.inner.hash_slow();
        let state = Arc::new(WatchRpcState {
            request: request.clone(),
            chain,
            fork,
            original_visible,
            processed: AtomicBool::new(false),
            account_nonce: AtomicU64::new(1),
            head: AtomicU64::new(107),
            finalized_height: AtomicU64::new(100),
            receipt: Mutex::new(Value::Null),
            fault: Mutex::new(RpcFault::None),
            manager: Mutex::new(Address::from([5; 20])),
            calls: Mutex::new(Vec::new()),
        });
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let state = server_state.clone();
                        clients.spawn(async move {
                            let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { return };
                            while let Some(Ok(frame)) = socket.next().await {
                                if let WsMessage::Text(text) = frame {
                                    let request: Value = serde_json::from_str(&text).unwrap();
                                    let response = state.response(&request);
                                    let silent = {
                                        let mut fault = state.fault.lock().unwrap();
                                        let silent = (*fault == RpcFault::SilentReceipt && request["method"] == "eth_getTransactionReceipt")
                                            || (*fault == RpcFault::SilentEvidence && request["method"] == "eth_call");
                                        if silent { *fault = RpcFault::None; }
                                        silent
                                    };
                                    if silent { continue; }
                                    if socket.send(WsMessage::Text(response.to_string().into())).await.is_err() { break }
                                } else if frame.is_close() { break }
                            }
                        });
                    }
                    joined = clients.join_next(), if !clients.is_empty() => {
                        if let Some(Err(error)) = joined { panic!("watcher RPC fixture failed: {error}") }
                    }
                }
            }
        });
        let api = match EthApi::new(
            &url,
            &format!("{:#x}", Address::from([2; 20])),
            Some(&format!("0x{}", "07".repeat(32))),
            None,
            None,
        )
        .await
        {
            Ok(api) => api,
            Err(error) => {
                server.abort();
                return Err(error.into());
            }
        };
        Ok(WatchRpc {
            api,
            state,
            server,
            _network: network,
        })
    }

    fn expect_completed(outcome: SubmissionOutcome) -> anyhow::Result<CompletionEvidence> {
        match outcome {
            SubmissionOutcome::Completed(evidence) => Ok(evidence),
            other => anyhow::bail!("expected original completion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn absent_original_never_completes_from_nonce_or_competing_receipt() -> anyhow::Result<()>
    {
        for (case, visible, processed, fault, other_receipt) in [
            (
                "absent original, finalized processed",
                false,
                true,
                RpcFault::None,
                false,
            ),
            ("competing transaction", false, true, RpcFault::None, true),
            ("pending original", true, false, RpcFault::None, false),
            ("receipt RPC failure", false, true, RpcFault::Receipt, false),
            (
                "missing finalized head",
                false,
                true,
                RpcFault::MissingFinality,
                false,
            ),
            (
                "invalid finalized header",
                false,
                true,
                RpcFault::InvalidFinality,
                false,
            ),
            (
                "conflicting finalized history",
                false,
                true,
                RpcFault::ConflictingFinality,
                false,
            ),
        ] {
            let rpc = watch_rpc(visible).await?;
            let request = rpc.state.request.clone();
            if fault == RpcFault::ConflictingFinality {
                assert!(
                    !rpc.api
                        .is_message_processed(request.message.nonce_be)
                        .await?
                );
            }
            rpc.state.processed.store(processed, Ordering::SeqCst);
            *rpc.state.fault.lock().unwrap() = fault;
            if other_receipt {
                *rpc.state.receipt.lock().unwrap() = rpc.state.receipt(TxHash::from([10; 32]));
            }
            assert!(
                tokio::time::timeout(
                    Duration::from_secs(2),
                    watch_tx(rpc.api.clone(), request, 8)
                )
                .await
                .is_err(),
                "{case}: uncertain original receipt must remain pending"
            );
            let calls = rpc.state.calls.lock().unwrap();

            assert!(!calls
                .iter()
                .any(|call| call["method"].as_str().unwrap().starts_with("eth_send")));
        }
        Ok(())
    }

    #[tokio::test]
    async fn original_receipt_requires_finalized_confirmations_and_token_effect(
    ) -> anyhow::Result<()> {
        for valid in [true, false] {
            let rpc = watch_rpc(true).await?;
            let request = rpc.state.request.clone();
            let signed = request.submission.as_ref().unwrap();
            let mut receipt =
                serde_json::to_value(evidence(&request.message, signed, true).receipt)?;
            receipt["blockHash"] = json!(rpc.state.chain[100].header.hash);
            for log in receipt["logs"].as_array_mut().unwrap() {
                log["blockHash"] = json!(rpc.state.chain[100].header.hash);
            }
            if !valid {
                receipt["logs"].as_array_mut().unwrap().pop();
            }
            *rpc.state.receipt.lock().unwrap() = receipt;
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(100),
                    watch_tx(rpc.api.clone(), request.clone(), 8)
                )
                .await
                .is_err(),
                "latest confirmations are not finality"
            );
            rpc.state.finalized_height.store(107, Ordering::SeqCst);
            let (_, outcome) = tokio::time::timeout(
                Duration::from_secs(1),
                watch_tx(rpc.api.clone(), request, 8),
            )
            .await?;
            if valid {
                assert!(expect_completed(outcome)?.token_delivery);
            } else {
                assert!(matches!(outcome, SubmissionOutcome::NeedsReconciliation {
                    diagnostic, nonce_disposition: None,
                } if diagnostic.contains("lacks exactly one")));
            }
        }
        Ok(())
    }
    #[tokio::test]
    async fn application_receipt_completes_without_a_token_manager() -> anyhow::Result<()> {
        let rpc = watch_rpc(true).await?;
        *rpc.state.manager.lock().unwrap() = Address::ZERO;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        let request = rpc.state.request.clone();
        let signed = request.submission.as_ref().unwrap();
        // Token-shaped application payloads remain applications without a token lane.
        let mut receipt = serde_json::to_value(evidence(&request.message, signed, false).receipt)?;
        receipt["blockHash"] = json!(rpc.state.chain[100].header.hash);
        receipt["logs"][0]["blockHash"] = json!(rpc.state.chain[100].header.hash);
        for case in ["valid", "missing", "wrong-message", "other-receipt"] {
            let mut candidate = receipt.clone();
            match case {
                "missing" => candidate["logs"] = json!([]),
                "wrong-message" => {
                    candidate["logs"][0]["data"] = json!(format!("0x{}", "00".repeat(128)));
                }
                "other-receipt" => {
                    candidate["logs"][0]["transactionHash"] = json!(B256::ZERO);
                }
                _ => {}
            }
            *rpc.state.receipt.lock().unwrap() = candidate;
            let (_, outcome) = tokio::time::timeout(
                Duration::from_secs(2),
                watch_tx(rpc.api.clone(), request.clone(), 8),
            )
            .await?;
            if case == "valid" {
                assert!(!expect_completed(outcome)?.token_delivery);
            } else {
                assert!(
                    matches!(outcome, SubmissionOutcome::NeedsReconciliation { .. }),
                    "{case}"
                );
            }
        }
        let calls = rpc.state.calls.lock().unwrap();
        assert!(!calls.iter().any(|call| {
            call["method"].as_str().unwrap().starts_with("eth_send")
                || (call["method"] == "eth_call"
                    && call["params"][0]
                        .get("input")
                        .or_else(|| call["params"][0].get("data"))
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .starts_with(&format!(
                            "0x{}",
                            hex::encode(IERC20Manager::isVftManagerCall::SELECTOR)
                        )))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn transient_receipt_observations_preserve_original_submission() -> anyhow::Result<()> {
        for fault in [
            RpcFault::DelayedReceipt,
            RpcFault::SilentReceipt,
            RpcFault::SilentEvidence,
        ] {
            let rpc = watch_rpc(true).await?;
            let request = rpc.state.request.clone();
            let signed = request.submission.as_ref().unwrap();
            let mut receipt =
                serde_json::to_value(evidence(&request.message, signed, true).receipt)?;
            receipt["blockHash"] = json!(rpc.state.chain[100].header.hash);
            for log in receipt["logs"].as_array_mut().unwrap() {
                log["blockHash"] = json!(rpc.state.chain[100].header.hash);
            }
            *rpc.state.receipt.lock().unwrap() = receipt;
            rpc.state.finalized_height.store(107, Ordering::SeqCst);
            rpc.state.processed.store(true, Ordering::SeqCst);
            *rpc.state.fault.lock().unwrap() = fault;
            let (saved, outcome) = tokio::time::timeout(
                Duration::from_secs(75),
                watch_tx(rpc.api.clone(), request.clone(), 8),
            )
            .await?;
            let completion = expect_completed(outcome)?;
            assert_eq!(completion.receipt.transaction_hash, request.tx_hash);
            assert!(completion.token_delivery);
            assert!(saved.submission == request.submission);
            {
                let calls = rpc.state.calls.lock().unwrap();

                assert!(!calls
                    .iter()
                    .any(|call| call["method"].as_str().unwrap().starts_with("eth_send")));
            }
            *rpc.state.fault.lock().unwrap() = RpcFault::Receipt;
            assert!(matches!(
                rpc.api.get_finalized_receipt(request.tx_hash).await,
                Err(ethereum_client::Error::ErrorInHTTPTransport(
                    alloy::transports::RpcError::ErrorResp(_)
                ))
            ));
        }
        Ok(())
    }

    fn set_finalized_revert(rpc: &WatchRpc) {
        let mut receipt = rpc.state.receipt(rpc.state.request.tx_hash);
        receipt["status"] = json!("0x0");
        *rpc.state.receipt.lock().unwrap() = receipt;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        rpc.state.account_nonce.store(2, Ordering::SeqCst);
        rpc.state.processed.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn finalized_original_revert_retains_strict_evidence_and_competitor_is_not_success(
    ) -> anyhow::Result<()> {
        let rpc = watch_rpc(false).await?;
        set_finalized_revert(&rpc);
        let request = rpc.state.request.clone();
        let signed = request.submission.as_ref().unwrap();
        let (_, outcome) = tokio::time::timeout(
            Duration::from_secs(2),
            watch_tx(rpc.api.clone(), request.clone(), 8),
        )
        .await?;
        let SubmissionOutcome::FinalizedRevert(failure) = outcome else {
            anyhow::bail!("expected finalized original revert")
        };
        assert_eq!(failure.receipt.transaction_hash, signed.hash);
        assert_eq!(
            failure.included_block_hash,
            rpc.state.chain[100].header.hash
        );
        assert_eq!(
            failure.finalized_block_hash,
            rpc.state.chain[107].header.hash
        );
        let restored: FinalizedFailureEvidence =
            serde_json::from_slice(&serde_json::to_vec(&failure)?)?;
        validate_failure(&request.message, signed, &restored)?;
        let (_, restarted) = tokio::time::timeout(
            Duration::from_secs(2),
            watch_tx(rpc.api.clone(), request.clone(), 8),
        )
        .await?;
        assert!(
            matches!(restarted, SubmissionOutcome::FinalizedRevert(evidence) if evidence.receipt.transaction_hash == signed.hash)
        );
        for case in [
            "success",
            "competing",
            "sender",
            "destination",
            "height",
            "hash",
            "missing-index",
            "unfinalized",
            "same-height-fork",
            "logs",
        ] {
            let mut value = serde_json::to_value(&failure)?;
            match case {
                "success" => value["receipt"]["status"] = json!("0x1"),
                "competing" => value["receipt"]["transactionHash"] = json!(B256::ZERO),
                "sender" => value["receipt"]["from"] = json!(Address::ZERO),
                "destination" => value["receipt"]["to"] = json!(Address::ZERO),
                "height" => value["included_block_number"] = json!(99),
                "hash" => value["included_block_hash"] = json!(B256::ZERO),
                "missing-index" => value["receipt"]["transactionIndex"] = Value::Null,
                "unfinalized" => value["finalized_block_number"] = json!(99),
                "same-height-fork" => value["finalized_block_number"] = json!(100),
                "logs" => {
                    value["receipt"]["logs"] =
                        serde_json::to_value(evidence(&request.message, signed, true).receipt)?
                            ["logs"]
                            .clone()
                }
                _ => unreachable!(),
            }
            let invalid: FinalizedFailureEvidence = serde_json::from_value(value)?;
            assert!(
                validate_failure(&request.message, signed, &invalid).is_err(),
                "{case}"
            );
        }
        for field in [
            "receipt",
            "included_block_number",
            "included_block_hash",
            "finalized_block_number",
            "finalized_block_hash",
        ] {
            let mut value = serde_json::to_value(&failure)?;
            value.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<FinalizedFailureEvidence>(value).is_err(),
                "missing {field}"
            );
        }
        let mut value = serde_json::to_value(&failure)?;
        value["unexpected"] = json!(true);
        assert!(serde_json::from_value::<FinalizedFailureEvidence>(value).is_err());
        let noncanonical = FinalizedTransactionReceipt {
            receipt: failure.receipt.clone(),
            included_block_number: failure.included_block_number,
            included_block_hash: failure.included_block_hash,
            finalized_block_number: 107,
            finalized_block_hash: B256::from([22; 32]),
        };
        assert!(
            failure_evidence(&rpc.api, &request.message, signed, noncanonical)
                .await
                .is_err()
        );
        assert!(!rpc
            .state
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["method"].as_str().unwrap().starts_with("eth_send")));
        Ok(())
    }

    #[tokio::test]
    async fn consumed_nonce_absence_is_held_not_original_failure_or_completion(
    ) -> anyhow::Result<()> {
        for processed in [false, true] {
            let rpc = watch_rpc(false).await?;
            rpc.state.account_nonce.store(2, Ordering::SeqCst);
            rpc.state.processed.store(processed, Ordering::SeqCst);
            let request = rpc.state.request.clone();
            let (_, outcome) = tokio::time::timeout(
                Duration::from_secs(2),
                watch_tx(rpc.api.clone(), request.clone(), 8),
            )
            .await?;
            let SubmissionOutcome::NeedsReconciliation {
                diagnostic,
                nonce_disposition: Some(snapshot),
            } = outcome
            else {
                anyhow::bail!("expected authenticated consumed-nonce hold");
            };
            assert!(diagnostic.contains("consumed nonce"));
            assert!(snapshot.pinned_nonce_consumed);
            assert_eq!(snapshot.message_processed, processed);
            assert_eq!(snapshot.block_hash, rpc.state.chain[100].header.hash);
            assert!(request.submission == rpc.state.request.submission);
            let calls = rpc.state.calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call["method"] == "eth_getTransactionReceipt")
                    .count(),
                2
            );
            assert!(!calls
                .iter()
                .any(|call| call["method"].as_str().unwrap().starts_with("eth_send")));
        }
        Ok(())
    }

    #[tokio::test]
    async fn saved_nonce_disposition_reauthenticates_original_pin_and_both_state_bits(
    ) -> anyhow::Result<()> {
        let rpc = watch_rpc(false).await?;
        rpc.state.account_nonce.store(2, Ordering::SeqCst);
        rpc.state.processed.store(true, Ordering::SeqCst);
        let saved = rpc
            .api
            .get_finalized_submission_snapshot(1, rpc.state.request.message.nonce_be)
            .await?
            .unwrap();
        let saved: FinalizedSubmissionSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        rpc.api
            .verify_finalized_submission_snapshot(1, rpc.state.request.message.nonce_be, &saved)
            .await?;
        for (consumed, processed) in [(false, true), (true, false)] {
            let changed = FinalizedSubmissionSnapshot {
                pinned_nonce_consumed: consumed,
                message_processed: processed,
                ..saved
            };
            assert!(rpc
                .api
                .verify_finalized_submission_snapshot(
                    1,
                    rpc.state.request.message.nonce_be,
                    &changed
                )
                .await
                .is_err());
        }
        let wrong_pin = FinalizedSubmissionSnapshot {
            block_hash: B256::from([22; 32]),
            ..saved
        };
        assert!(rpc
            .api
            .verify_finalized_submission_snapshot(1, rpc.state.request.message.nonce_be, &wrong_pin)
            .await
            .is_err());
        *rpc.state.fault.lock().unwrap() = RpcFault::MissingFinality;
        assert!(rpc
            .api
            .verify_finalized_submission_snapshot(1, rpc.state.request.message.nonce_be, &saved)
            .await
            .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn signed_original_payload_is_validated_before_every_outcome() -> anyhow::Result<()> {
        let rpc = watch_rpc(false).await?;
        set_finalized_revert(&rpc);
        let mut request = rpc.state.request.clone();
        request.message.payload[0] ^= 1;
        let (_, outcome) = watch_tx(rpc.api.clone(), request, 8).await;
        assert!(matches!(
            outcome,
            SubmissionOutcome::NeedsReconciliation {
                nonce_disposition: None,
                ..
            }
        ));
        assert!(!rpc
            .state
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["method"] == "eth_getTransactionReceipt"
                || call["method"] == "eth_getTransactionCount"));
        let request = &rpc.state.request;
        let signed = request.submission.as_ref().unwrap();
        let mut wrong = request.message.clone();
        wrong.nonce_be[0] ^= 1;
        assert!(
            validate_completion(&wrong, signed, &evidence(&request.message, signed, true)).is_err()
        );
        let finalized = rpc
            .api
            .get_finalized_receipt(request.tx_hash)
            .await?
            .unwrap();
        assert!(failure_evidence(&rpc.api, &wrong, signed, finalized)
            .await
            .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn status_accounting_counts_finalized_failure_once_and_never_holds() -> anyhow::Result<()>
    {
        let rpc = watch_rpc(false).await?;
        set_finalized_revert(&rpc);
        let fetcher = StatusFetcher::new(rpc.api.clone(), 8);
        let failed = fetcher.metrics.total_failed_txs.clone();
        let pending = fetcher.metrics.pending_tx_count.clone();
        let mut io = fetcher.spawn();
        let request = rpc.state.request.clone();
        for _ in 0..2 {
            assert!(io.send_request(
                request.tx_uuid,
                request.tx_hash,
                request.message.clone(),
                request.submission.clone()
            ));
        }
        let (_, outcome) = tokio::time::timeout(Duration::from_secs(2), io.recv_message())
            .await?
            .unwrap();
        assert!(matches!(outcome, SubmissionOutcome::FinalizedRevert(_)));
        assert_eq!(failed.get(), 1);
        assert_eq!(pending.get(), 0);
        let mut invalid = request.message.clone();
        invalid.payload[0] ^= 1;
        assert!(io.send_request(
            Uuid::new_v4(),
            request.tx_hash,
            invalid,
            request.submission.clone()
        ));
        let (_, outcome) = tokio::time::timeout(Duration::from_secs(2), io.recv_message())
            .await?
            .unwrap();
        assert!(matches!(
            outcome,
            SubmissionOutcome::NeedsReconciliation { .. }
        ));
        assert_eq!(failed.get(), 1);
        assert_eq!(pending.get(), 0);
        *rpc.state.receipt.lock().unwrap() = Value::Null;
        assert!(io.send_request(
            Uuid::new_v4(),
            request.tx_hash,
            request.message.clone(),
            request.submission.clone()
        ));
        let (_, outcome) = tokio::time::timeout(Duration::from_secs(2), io.recv_message())
            .await?
            .unwrap();
        assert!(matches!(
            outcome,
            SubmissionOutcome::NeedsReconciliation {
                nonce_disposition: Some(_),
                ..
            }
        ));
        assert_eq!(failed.get(), 1);
        set_finalized_revert(&rpc);
        assert!(io.send_request(
            Uuid::new_v4(),
            request.tx_hash,
            request.message.clone(),
            request.submission.clone()
        ));
        let (_, outcome) = tokio::time::timeout(Duration::from_secs(2), io.recv_message())
            .await?
            .unwrap();
        assert!(matches!(outcome, SubmissionOutcome::FinalizedRevert(_)));
        assert_eq!(failed.get(), 1);
        assert!(io.send_request(
            request.tx_uuid,
            request.tx_hash,
            request.message,
            request.submission
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), io.recv_message())
                .await
                .is_err()
        );
        assert_eq!(failed.get(), 1);
        Ok(())
    }

    // Local-only smoke: real queue/handler bytecode, seeded disposable queue roots.
    // This does not qualify consensus, deployment governance, or public finality.
    #[tokio::test]
    #[ignore = "requires anvil and existing Foundry artifacts"]
    async fn disposable_anvil_handler_original_receipt_and_replay() -> anyhow::Result<()> {
        use alloy::{
            network::TransactionBuilder, rpc::types::TransactionRequest, sol_types::SolValue,
        };
        use std::process::{Command, Stdio};
        struct Node(std::process::Child);
        impl Drop for Node {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = socket.local_addr()?.port();
        drop(socket);
        let _node = Node(
            Command::new("anvil")
                .args([
                    "--port",
                    &port.to_string(),
                    "--chain-id",
                    "560048",
                    "--silent",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let url = format!("ws://127.0.0.1:{port}");
        let key = format!("0x{}", hex::encode([7; 32]));
        let queue = Address::from([2; 20]);
        let governance = Address::from([6; 20]);
        let manager = Address::from([5; 20]);
        let mut api = None;
        for _ in 0..50 {
            match EthApi::new_with_retries(
                &url,
                &format!("{queue:#x}"),
                Some(&key),
                Some(0),
                Some(Duration::from_millis(10)),
                None,
                None,
            )
            .await
            {
                Ok(connected) => {
                    api = Some(connected);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        let api = api.ok_or_else(|| anyhow::anyhow!("disposable Anvil failed to start"))?;
        let provider = api.raw_provider();
        let sender = Address::from_slice(api.sender_address().as_bytes());
        let _: Value = provider
            .client()
            .request("anvil_setBalance", (sender, "0x1000000000000000000000000"))
            .await?;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ethereum");
        // Two existing artifact formats are read verbatim; no generated files are altered.
        let artifact: Value = serde_json::from_slice(&std::fs::read(
            root.join("out/MessageQueue.sol/MessageQueue.json"),
        )?)?;
        let code = artifact["deployedBytecode"]["object"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("build MessageQueue artifacts first"))?;
        let _: Value = provider
            .client()
            .request("anvil_setCode", (queue, code))
            .await?;
        let artifact: Value = serde_json::from_slice(&std::fs::read(
            root.join("out/GovernanceAdmin.sol/GovernanceAdmin.json"),
        )?)?;
        let code = artifact["deployedBytecode"]["object"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("build GovernanceAdmin artifacts first"))?;
        let _: Value = provider
            .client()
            .request("anvil_setCode", (governance, code))
            .await?;
        let word = |address: Address| {
            B256::from(U256::from_be_slice(address.as_slice()).to_be_bytes::<32>())
        };
        let _: Value = provider
            .client()
            .request("anvil_setStorageAt", (queue, B256::ZERO, word(governance)))
            .await?;
        let _: Value = provider
            .client()
            .request(
                "anvil_setStorageAt",
                (
                    queue,
                    B256::from(U256::from(1).to_be_bytes::<32>()),
                    word(governance),
                ),
            )
            .await?;
        let _: Value = provider
            .client()
            .request(
                "anvil_setStorageAt",
                (
                    governance,
                    B256::from(U256::from(3).to_be_bytes::<32>()),
                    word(manager),
                ),
            )
            .await?;
        let artifact_path = root
            .join("../js/bridge-js/js-test/contracts/out/MessageHandler.sol/MessageHandler.json");
        let artifact: Value = serde_json::from_slice(&std::fs::read(artifact_path)?)?;
        let code = artifact["bytecode"]["object"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("build MessageHandler artifacts first"))?;
        let source = B256::from([8; 32]);
        let mut deployment = hex::decode(code.trim_start_matches("0x"))?;
        deployment.extend((queue, source, sender).abi_encode());
        let deployed = provider
            .send_transaction(
                TransactionRequest::default()
                    .with_from(sender)
                    .with_deploy_code(deployment)
                    .with_max_fee_per_gas(2000000000)
                    .with_max_priority_fee_per_gas(1),
            )
            .await?
            .get_receipt()
            .await?;
        let handler = deployed
            .contract_address
            .ok_or_else(|| anyhow::anyhow!("handler deployment reverted"))?;
        let message = Message {
            nonce_be: [7; 32],
            source: source.0,
            destination: handler.into_array(),
            payload: vec![9; 64],
        };
        let map = |key: U256, slot: u64| {
            alloy::primitives::keccak256((key, U256::from(slot)).abi_encode())
        };
        let _: Value = provider
            .client()
            .request(
                "anvil_setStorageAt",
                (
                    queue,
                    map(U256::from(43), 11),
                    B256::from(message_hash(&message)),
                ),
            )
            .await?;
        let _: Value = provider
            .client()
            .request(
                "anvil_setStorageAt",
                (
                    queue,
                    map(U256::from(43), 15),
                    B256::from(U256::from(1).to_be_bytes::<32>()),
                ),
            )
            .await?;

        let signed = signed_message(&message).await;
        let receipt = provider
            .send_raw_transaction(&signed.raw_transaction)
            .await?
            .get_receipt()
            .await?;
        assert!(receipt.status(), "actual queue/handler dispatch reverted");
        let _: Value = provider.client().request("anvil_mine", ("0x80",)).await?;
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: signed.hash,
            message: message.clone(),
            submission: Some(signed.clone()),
        };
        let (_, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            watch_tx(api.clone(), request.clone(), 8),
        )
        .await?;
        let evidence = expect_completed(outcome)?;
        assert!(!evidence.token_delivery);
        let restored: CompletionEvidence = serde_json::from_slice(&serde_json::to_vec(&evidence)?)?;
        validate_completion(&message, &signed, &restored)?;
        let (_, restarted) =
            tokio::time::timeout(Duration::from_secs(10), watch_tx(api.clone(), request, 8))
                .await?;
        assert_eq!(
            expect_completed(restarted)?.receipt.transaction_hash,
            signed.hash
        );
        alloy::sol! { #[sol(rpc)] interface HandlerView { function received(bytes32 applicationId) external view returns (bool); } }
        assert!(
            HandlerView::new(handler, provider)
                .received(B256::from([9; 32]))
                .call()
                .await?
        );
        let replay = signed_message_with_nonce(&message, 2).await;
        let reverted = provider
            .send_raw_transaction(&replay.raw_transaction)
            .await?
            .get_receipt()
            .await?;
        assert!(!reverted.status(), "actual queue replay must revert");
        let _: Value = provider.client().request("anvil_mine", ("0x80",)).await?;
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: replay.hash,
            message: message.clone(),
            submission: Some(replay),
        };
        let (_, replay) =
            tokio::time::timeout(Duration::from_secs(10), watch_tx(api.clone(), request, 8))
                .await?;
        assert!(matches!(replay, SubmissionOutcome::FinalizedRevert(_)));
        let absent = signed_message_with_nonce(&message, 3).await;
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: absent.hash,
            message,
            submission: Some(absent),
        };
        let absent = tokio::time::timeout(Duration::from_secs(1), watch_tx(api, request, 8)).await;
        assert!(absent.is_err());
        Ok(())
    }
}
