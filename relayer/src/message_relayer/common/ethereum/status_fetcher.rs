use crate::{common::BASE_RETRY_DELAY, lane_policy::LanePolicy, message_relayer::common::message_hash};
use alloy::{
    primitives::{Address, B256, U256},
    sol_types::{SolCall, SolEvent},
};
use ethereum_client::{
    abi::{IERC20Manager, IMessageQueue},
    inclusion::{verify_indexed_value, RecordedReceiptProof},
    EthApi, FinalityCheckpoint, FinalizedSubmissionSnapshot, FinalizedTransactionReceipt,
    PreparedContentMessage, TxHash,
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
    lane_policy: Option<LanePolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionEvidence {
    pub receipt: alloy::rpc::types::TransactionReceipt,
    pub finalized_block_number: u64,
    pub finalized_block_hash: B256,
    pub token_delivery: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inclusion: Option<RecordedReceiptProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<FinalityCheckpoint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizedFailureEvidence {
    pub receipt: alloy::rpc::types::TransactionReceipt,
    pub included_block_number: u64,
    pub included_block_hash: B256,
    pub finalized_block_number: u64,
    pub finalized_block_hash: B256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inclusion: Option<RecordedReceiptProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<FinalityCheckpoint>,
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
        lane_policy: Option<LanePolicy>,
    ) -> bool {
        self.requests
            .send(TrackedRequest {
                tx_uuid,
                tx_hash,
                message,
                submission,
                lane_policy,
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

pub(super) fn original_call(
    message: &Message,
    signed: &PreparedContentMessage,
) -> anyhow::Result<IMessageQueue::processMessageCall> {
    use alloy::consensus::Transaction;
    let (envelope, _) = ethereum_client::prepared_content_message_transaction(signed)?;
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

/// Check retained inclusion and original identity. The caller must requalify
/// every historical checkpoint fence under Beacon authority before progress.
pub fn validate_receipt_authentication(
    signed: &PreparedContentMessage,
    receipt: &alloy::rpc::types::TransactionReceipt,
    inclusion: &RecordedReceiptProof,
    checkpoint: &FinalityCheckpoint,
) -> anyhow::Result<()> {
    use alloy::eips::Encodable2718;
    ethereum_client::prepared_content_message_transaction(signed)?;
    anyhow::ensure!(
        matches!((checkpoint.schema_version(), checkpoint.policy()),
            (1, "independent-two-el-v1") | (2, "beacon-sync-committee-v1"))
            && checkpoint.chain_id() == signed.chain_id
            && checkpoint.genesis_hash() != B256::ZERO
            && inclusion.genesis == checkpoint.genesis_hash()
            && inclusion.raw_transaction == signed.raw_transaction
            && receipt.transaction_hash == signed.hash
            && receipt.from == signed.sender
            && receipt.to == Some(signed.contract)
            && receipt.block_number == Some(inclusion.header.number)
            && receipt.block_hash == Some(inclusion.header.hash_slow())
            && receipt.transaction_index == Some(inclusion.index)
            && inclusion.header.number <= checkpoint.common_header().number,
        "HOLD: receipt proof/checkpoint differs from signed original identity"
    );
    anyhow::ensure!(
        inclusion.header.number != checkpoint.common_header().number
            || &inclusion.header == checkpoint.common_header(),
        "HOLD: same-height checkpoint headers disagree"
    );
    if let Some(legacy) = checkpoint.legacy() {
        for header in [&legacy.primary_finalized_header, &legacy.witness_finalized_header] {
            anyhow::ensure!(header.number >= legacy.common_header.number
                && (header.number != legacy.common_header.number || header == &legacy.common_header),
                "HOLD: historical checkpoint fences conflict");
        }
    }
    anyhow::ensure!(
        receipt.inner.clone().into_primitives_receipt() == inclusion.receipt,
        "HOLD: saved receipt contents differ from included envelope"
    );
    verify_indexed_value(
        inclusion.header.transactions_root,
        inclusion.index,
        &inclusion.raw_transaction,
        &inclusion.transaction_proof,
    )?;
    verify_indexed_value(
        inclusion.header.receipts_root,
        inclusion.index,
        &inclusion.receipt.encoded_2718(),
        &inclusion.receipt_proof,
    )?;
    let mut previous = None;
    for log in receipt.inner.logs() {
        let index = log
            .log_index
            .ok_or_else(|| anyhow::anyhow!("HOLD: log index missing"))?;
        anyhow::ensure!(
            !log.removed
                && log.transaction_hash == Some(signed.hash)
                && log.transaction_index == Some(inclusion.index)
                && log.block_number == receipt.block_number
                && log.block_hash == receipt.block_hash
                && previous.is_none_or(|last: u64| last.checked_add(1) == Some(index)),
            "HOLD: receipt log coordinates conflict"
        );
        previous = Some(index);
    }
    Ok(())
}

fn validate_optional_authentication(
    signed: &PreparedContentMessage,
    receipt: &alloy::rpc::types::TransactionReceipt,
    inclusion: Option<&RecordedReceiptProof>,
    checkpoint: Option<&FinalityCheckpoint>,
    finalized_number: u64,
    finalized_hash: B256,
) -> anyhow::Result<()> {
    match (inclusion, checkpoint) {
        (None, None) => Ok(()), // Legacy structural loading, never a new terminal verdict.
        (Some(inclusion), Some(checkpoint)) => {
            validate_receipt_authentication(signed, receipt, inclusion, checkpoint)?;
            anyhow::ensure!(
                finalized_number <= checkpoint.common_header().number
                    && (finalized_number != checkpoint.common_header().number
                        || checkpoint.common_header().hash_slow() == finalized_hash),
                "HOLD: retained finality coordinates differ from checkpoint"
            );
            Ok(())
        }
        _ => anyhow::bail!("HOLD: incomplete receipt authentication"),
    }
}

pub fn validate_completion(
    message: &Message,
    signed: &PreparedContentMessage,
    evidence: &CompletionEvidence,
) -> anyhow::Result<()> {
    let call = original_call(message, signed)?;
    validate_optional_authentication(
        signed,
        &evidence.receipt,
        evidence.inclusion.as_ref(),
        evidence.checkpoint.as_ref(),
        evidence.finalized_block_number,
        evidence.finalized_block_hash,
    )?;
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
    validate_optional_authentication(
        signed,
        &evidence.receipt,
        evidence.inclusion.as_ref(),
        evidence.checkpoint.as_ref(),
        evidence.finalized_block_number,
        evidence.finalized_block_hash,
    )?;
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
    anyhow::ensure!(finalized.checkpoint.beacon().is_some(), "HOLD: new failure evidence requires Beacon authority");
    let evidence = FinalizedFailureEvidence {
        receipt: finalized.receipt,
        included_block_number: finalized.included_block_number,
        included_block_hash: finalized.included_block_hash,
        finalized_block_number: finalized.finalized_block_number,
        finalized_block_hash: finalized.finalized_block_hash,
        inclusion: Some(finalized.inclusion),
        checkpoint: Some(finalized.checkpoint),
    };
    validate_failure(message, signed, &evidence)?;
    let mut view = api.verified_finalized_view().await?;
    view.verify_checkpoint(api.raw_provider(), evidence.checkpoint.as_ref().unwrap())
        .await?;
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
    policy: &LanePolicy,
) -> anyhow::Result<CompletionEvidence> {
    anyhow::ensure!(finalized.checkpoint.beacon().is_some(), "HOLD: new completion evidence requires Beacon authority");
    policy.validate_for_api(api, message)?;
    policy.validate(message, signed)?;
    validate_receipt_authentication(
        signed,
        &finalized.receipt,
        &finalized.inclusion,
        &finalized.checkpoint,
    )?;
    let mut view = api.verified_finalized_view().await?;
    view.verify_checkpoint(api.raw_provider(), &finalized.checkpoint)
        .await?;
    view.verify_blocks(
        api.raw_provider(),
        &BTreeMap::from([(
            finalized.included_block_number,
            finalized.included_block_hash,
        )]),
    )
    .await?;
    let token_delivery = policy.token_delivery_required;
    view.recheck(api.raw_provider()).await?;
    let evidence = CompletionEvidence {
        receipt: finalized.receipt,
        finalized_block_number: finalized.finalized_block_number,
        finalized_block_hash: finalized.finalized_block_hash,
        token_delivery,
        inclusion: Some(finalized.inclusion),
        checkpoint: Some(finalized.checkpoint),
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
        let Some(policy) = request.lane_policy.as_ref() else {
            return (request, hold("HOLD: original has no approved immutable lane policy".into()));
        };
        if let Err(error) = policy.validate_for_api(&api, &request.message).and_then(|_| policy.validate(&request.message, signed)) {
            return (request, hold(error.to_string()));
        }
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
            match api.get_finalized_receipt(request.tx_hash).await {
                Ok(Some(receipt)) => {
                    if finalized_receipt_has_required_confirmations(
                        receipt.included_block_number,
                        receipt.finalized_block_number,
                        confirmations,
                    ) {
                        let result = if receipt.receipt.status() {
                            completion_evidence(&api, &request.message, signed, receipt, policy)
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
                        .get_finalized_submission_snapshot(&signed.raw_transaction)
                        .await
                    {
                        Ok(Some(snapshot)) if snapshot.pinned_nonce_consumed() => {
                            match api
                                .verify_finalized_submission_snapshot(
                                    &signed.raw_transaction,
                                    &snapshot,
                                )
                                .await
                            {
                                Ok(snapshot) => {
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
        Some(ethereum_client::Error::FinalizedAncestryPending | ethereum_client::Error::ErrorFetchingBlock)
    ) || ethereum_client::is_transient_rpc_error(error)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloy::providers::Provider;
    use ethereum_client::beacon_finality::{
        test_fixtures::LocalBeaconFixture, LegacyFinalityCheckpoint,
    };

    alloy::sol! {
        interface QueueGovernance {
            function erc20Manager() external view returns (address);
        }
    }

    pub(crate) fn lane_policy(
        message: &Message,
        signed: &PreparedContentMessage,
        genesis: B256,
        token_delivery: bool,
    ) -> LanePolicy {
        LanePolicy {
            approval_sha256: B256::repeat_byte(1),
            chain_id: signed.chain_id,
            genesis_hash: genesis,
            queue: signed.contract,
            source_manager: B256::from(message.source),
            destination: Address::from(message.destination),
            evm_manager: token_delivery.then_some(Address::from(message.destination)),
            token_delivery_required: token_delivery,
            original_hash: None,
        }
    }

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
            chain_id: 31337,
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
            chain_id: 31337,
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
        let (_, receipt, inclusion, checkpoint) =
            authenticated_fixture(signed, receipt, signed.hash.as_slice(), 100, 107);
        CompletionEvidence {
            receipt,
            finalized_block_number: 107,
            finalized_block_hash: checkpoint.common_header().hash_slow(),
            token_delivery,
            inclusion: Some(inclusion),
            checkpoint: Some(checkpoint),
        }
    }

    pub(crate) fn reverted_evidence(
        message: &Message,
        signed: &PreparedContentMessage,
    ) -> FinalizedFailureEvidence {
        let mut receipt = serde_json::to_value(evidence(message, signed, false).receipt).unwrap();
        receipt["status"] = json!("0x0");
        receipt["logs"] = json!([]);
        let (_, receipt, inclusion, checkpoint) = authenticated_fixture(
            signed,
            serde_json::from_value(receipt).unwrap(),
            signed.hash.as_slice(),
            100,
            107,
        );
        FinalizedFailureEvidence {
            receipt,
            included_block_number: inclusion.header.number,
            included_block_hash: inclusion.header.hash_slow(),
            finalized_block_number: checkpoint.common_header().number,
            finalized_block_hash: checkpoint.common_header().hash_slow(),
            inclusion: Some(inclusion),
            checkpoint: Some(checkpoint),
        }
    }

    pub(crate) fn authenticated_fixture(
        signed: &PreparedContentMessage,
        receipt: alloy::rpc::types::TransactionReceipt,
        tag: &[u8],
        included: u64,
        head: u64,
    ) -> (
        Vec<Block>,
        alloy::rpc::types::TransactionReceipt,
        RecordedReceiptProof,
        FinalityCheckpoint,
    ) {
        use alloy::{
            consensus::{
                proofs::{calculate_receipt_root, calculate_transaction_root},
                transaction::Recovered,
                TxEnvelope,
            },
            eips::Decodable2718,
            rpc::types::{BlockTransactions, Transaction},
        };
        let transaction = TxEnvelope::decode_2718(&mut signed.raw_transaction.as_slice()).unwrap();
        let envelope = receipt.clone().into_primitives_receipt().inner;
        let mut parent = B256::ZERO;
        let (initial_nonce, transition, consumed_nonce) =
            if signed.nonce >= 36 { (36, 2, 38) } else { (1, 101, 2) };
        let initial_root = ethereum_client::beacon_finality::test_fixtures::single_account_proof(
            signed.sender, initial_nonce,
        ).0;
        let consumed_root = ethereum_client::beacon_finality::test_fixtures::single_account_proof(
            signed.sender, consumed_nonce,
        ).0;
        let chain: Vec<Block> = (0..=head)
            .map(|number| {
                let mut block: Block = Block::default();
                block.header.inner.number = number;
                block.header.inner.parent_hash = parent;
                block.header.inner.timestamp = number * 12;
                block.header.inner.extra_data = Bytes::copy_from_slice(tag);
                block.header.inner.state_root = if number < transition { initial_root } else { consumed_root };
                if number == included {
                    block.header.inner.transactions_root =
                        calculate_transaction_root(std::slice::from_ref(&transaction));
                    block.header.inner.receipts_root =
                        calculate_receipt_root(std::slice::from_ref(&envelope));
                }
                block.header.hash = block.header.inner.hash_slow();
                if number == included {
                    block.transactions = BlockTransactions::Full(vec![Transaction {
                        inner: Recovered::new_unchecked(transaction.clone(), signed.sender),
                        block_hash: Some(block.header.hash),
                        block_number: Some(number),
                        transaction_index: Some(0),
                        effective_gas_price: Some(1),
                    }]);
                }
                parent = block.header.hash;
                block
            })
            .collect();
        let header = chain[included as usize].header.inner.clone();
        let mut value = serde_json::to_value(receipt).unwrap();
        value["blockHash"] = json!(header.hash_slow());
        value["blockNumber"] = json!(format!("0x{included:x}"));
        value["transactionIndex"] = json!("0x0");
        for (index, log) in value["logs"].as_array_mut().unwrap().iter_mut().enumerate() {
            log["blockHash"] = json!(header.hash_slow());
            log["blockNumber"] = json!(format!("0x{included:x}"));
            log["transactionIndex"] = json!("0x0");
            log["logIndex"] = json!(format!("0x{index:x}"));
        }
        let receipt = serde_json::from_value(value).unwrap();
        let receipt_proof =
            ethereum_common::utils::generate_merkle_proof(0, &[(0, envelope.clone())]).unwrap();
        let inclusion = RecordedReceiptProof {
            genesis: chain[0].header.hash,
            transaction_proof: ethereum_client::inclusion::transaction_proof(
                &[transaction],
                0,
                header.transactions_root,
            )
            .unwrap(),
            header,
            index: 0,
            raw_transaction: signed.raw_transaction.clone(),
            receipt: envelope,
            receipt_proof: receipt_proof.proof,
        };
        // Historical structure only; live watchers acquire signed schema-2 evidence.
        let checkpoint = FinalityCheckpoint::Legacy(LegacyFinalityCheckpoint {
            schema_version: 1,
            policy: "independent-two-el-v1".into(),
            chain_id: signed.chain_id,
            genesis_hash: chain[0].header.hash,
            common_header: chain[head as usize].header.inner.clone(),
            primary_finalized_header: chain[head as usize].header.inner.clone(),
            witness_finalized_header: chain[head as usize].header.inner.clone(),
        });
        (chain, receipt, inclusion, checkpoint)
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
        Governance,
        Manager,
        Token,
        Nonce,
        Processed,
        WrongChain,
        ChangedPin,
    }

    struct WatchRpcState {
        request: TrackedRequest,
        second: TrackedRequest,
        second_receipt: Mutex<Value>,
        canonical_second: Value,
        chain: Vec<Block>,
        fork: Block,
        original_visible: bool,
        processed: AtomicBool,
        account_nonce: AtomicU64,
        head: AtomicU64,
        finalized_height: AtomicU64,
        receipt: Mutex<Value>,
        canonical_receipt: Value,
        witness_fault: Mutex<RpcFault>,
        witness_height: Mutex<Option<u64>>,
        pin_changed: AtomicBool,
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

        fn response(&self, request: &Value, witness: bool) -> Value {
            self.calls.lock().unwrap().push(request.clone());
            let fault = if witness {
                *self.witness_fault.lock().unwrap()
            } else {
                *self.fault.lock().unwrap()
            };
            let params = &request["params"];
            let result = match request["method"].as_str().unwrap() {
                "eth_chainId" => Ok(json!(if fault == RpcFault::WrongChain {
                    "0x1"
                } else {
                    "0x7a69"
                })),
                "eth_blockNumber" => Ok(json!(format!("0x{:x}", self.head.load(Ordering::SeqCst)))),
                "eth_getBlockReceipts" => {
                    let hash = params[0].get("blockHash").unwrap_or(&params[0]);
                    if hash == &json!(self.chain[101].header.hash) {
                        Ok(json!([self.canonical_second]))
                    } else {
                        assert_eq!(hash, &json!(self.chain[100].header.hash));
                        Ok(json!([self.canonical_receipt]))
                    }
                }
                "eth_getTransactionReceipt" if fault == RpcFault::Receipt => {
                    Err("receipt RPC unavailable")
                }
                "eth_getTransactionReceipt" if fault == RpcFault::DelayedReceipt => {
                    *self.fault.lock().unwrap() = RpcFault::None;
                    Ok(Value::Null)
                }
                "eth_getTransactionReceipt" => {
                    if params[0] == json!(self.second.tx_hash) {
                        Ok(self.second_receipt.lock().unwrap().clone())
                    } else {
                        assert_eq!(params[0], json!(self.request.tx_hash));
                        Ok(self.receipt.lock().unwrap().clone())
                    }
                }
                "eth_getProof" => {
                    let signed = self.request.submission.as_ref().unwrap();
                    assert_eq!(params[0], json!(signed.sender));
                    assert_eq!(params[2]["requireCanonical"], json!(true));
                    let block = self.chain.iter().find(|block|
                        json!(block.header.hash) == params[2]["blockHash"]
                    ).unwrap();
                    let nonce = if block.header.number < 101 { 1 } else { 2 };
                    let (_, mut proof) = ethereum_client::beacon_finality::test_fixtures::single_account_proof(
                        signed.sender, nonce,
                    );
                    if fault == RpcFault::Nonce { proof.nonce += 1; }
                    Ok(json!(proof))
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
                            + u64::from(fault == RpcFault::Nonce)
                    )))
                }
                "eth_getTransactionByHash" => Ok(if self.original_visible {
                    let alloy::rpc::types::BlockTransactions::Full(transactions) =
                        &self.chain[100].transactions
                    else {
                        panic!("fixture transaction is missing");
                    };
                    let mut transaction = json!(transactions[0]);
                    transaction["blockHash"] = Value::Null;
                    transaction["blockNumber"] = Value::Null;
                    transaction["transactionIndex"] = Value::Null;
                    transaction
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
                                &self.chain[if witness {
                                    self.witness_height
                                        .lock()
                                        .unwrap()
                                        .unwrap_or(self.finalized_height.load(Ordering::SeqCst))
                                } else {
                                    self.finalized_height.load(Ordering::SeqCst)
                                } as usize],
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
                        let mut block = self.chain[number as usize].clone();
                        if witness
                            && self.pin_changed.load(Ordering::SeqCst)
                            && number == self.finalized_height.load(Ordering::SeqCst)
                        {
                            block.header.inner.extra_data =
                                Bytes::from_static(b"changed after eth_call");
                            block.header.hash = block.header.inner.hash_slow();
                        }
                        Ok(serde_json::to_value(block).unwrap())
                    }
                }
                "eth_getBlockByHash" => Ok(self
                    .chain
                    .iter()
                    .find(|block| json!(block.header.hash) == params[0])
                    .map(|block| serde_json::to_value(block).unwrap())
                    .unwrap_or(Value::Null)),
                "eth_call" => {
                    assert_eq!(params[1]["requireCanonical"], json!(true));
                    assert!(self
                        .chain
                        .iter()
                        .any(|block| json!(block.header.hash) == params[1]["blockHash"]));
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
                        let governance = if fault == RpcFault::Governance {
                            Address::ZERO
                        } else {
                            Address::from([6; 20])
                        };
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", U256::from_be_slice(governance.as_slice()))});
                    }
                    if data
                        == format!(
                            "0x{}",
                            hex::encode(QueueGovernance::erc20ManagerCall::SELECTOR)
                        )
                    {
                        let manager = if fault == RpcFault::Manager {
                            Address::ZERO
                        } else {
                            *self.manager.lock().unwrap()
                        };
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", U256::from_be_slice(manager.as_slice()))});
                    }
                    if data.starts_with(&format!(
                        "0x{}",
                        hex::encode(IERC20Manager::isVftManagerCall::SELECTOR)
                    )) {
                        if fault == RpcFault::ChangedPin {
                            self.pin_changed.store(true, Ordering::SeqCst);
                        }
                        return json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", u8::from(fault != RpcFault::Token))});
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
                        u8::from(
                            self.processed.load(Ordering::SeqCst) ^ (fault == RpcFault::Processed)
                        )
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

    pub(crate) struct WatchRpc {
        pub(crate) api: EthApi,
        state: Arc<WatchRpcState>,
        server: tokio::task::JoinHandle<()>,
        _network: tokio::sync::MutexGuard<'static, ()>,
        _beacon: LocalBeaconFixture,
    }
    impl WatchRpc {
        pub(crate) fn advance_finalized_to(&self, number: u64) {
            assert!((number as usize) < self.state.chain.len());
            self.state.finalized_height.store(number, Ordering::SeqCst);
        }
    }


    impl Drop for WatchRpc {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn watch_rpc(original_visible: bool) -> anyhow::Result<WatchRpc> {
        watch_rpc_with_policy(original_visible, true).await
    }

    pub(crate) async fn watch_rpc_with_policy(
        original_visible: bool,
        token_delivery: bool,
    ) -> anyhow::Result<WatchRpc> {
        let network = WATCH_RPC_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}", listener.local_addr()?);
        let witness_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let witness_url = format!("ws://{}", witness_listener.local_addr()?);
        let message = Message {
            nonce_be: [7; 32],
            source: [8; 32],
            destination: [5; 20],
            payload: vec![6; 104],
        };
        let signed = signed_message(&message).await;
        let mut request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: signed.hash,
            message,
            submission: Some(signed),
            lane_policy: None,
        };
        let signed = request.submission.as_ref().unwrap();
        let canonical = if original_visible {
            evidence(&request.message, signed, token_delivery).receipt
        } else {
            reverted_evidence(&request.message, signed).receipt
        };
        let (mut chain, canonical, _, _) =
            authenticated_fixture(signed, canonical, request.tx_uuid.as_bytes(), 100, 107);
        let second_message = Message {
            nonce_be: [9; 32],
            ..request.message.clone()
        };
        let second_signed = signed_message_with_nonce(&second_message, 2).await;
        let mut second = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: second_signed.hash,
            message: second_message,
            submission: Some(second_signed),
            lane_policy: None,
        };
        let second_signed = second.submission.as_ref().unwrap();
        let (second_chain, second_receipt, _, _) = authenticated_fixture(
            second_signed,
            evidence(&second.message, second_signed, token_delivery).receipt,
            request.tx_uuid.as_bytes(),
            101,
            107,
        );
        chain[101] = second_chain[101].clone();
        let mut parent = chain[100].header.hash;
        for block in &mut chain[101..] {
            block.header.inner.parent_hash = parent;
            block.header.hash = block.header.inner.hash_slow();
            if let alloy::rpc::types::BlockTransactions::Full(transactions) =
                &mut block.transactions
            {
                for transaction in transactions {
                    transaction.block_hash = Some(block.header.hash);
                }
            }
            parent = block.header.hash;
        }
        let mut canonical_second = serde_json::to_value(second_receipt)?;
        canonical_second["blockHash"] = json!(chain[101].header.hash);
        for log in canonical_second["logs"].as_array_mut().unwrap() {
            log["blockHash"] = json!(chain[101].header.hash);
        }
        request.lane_policy = Some(lane_policy(
            &request.message, request.submission.as_ref().unwrap(), chain[0].header.hash, true,
        ));
        second.lane_policy = Some(lane_policy(
            &second.message, second.submission.as_ref().unwrap(), chain[0].header.hash, true,
        ));
        let mut fork = chain[100].clone();
        fork.header.inner.extra_data = Bytes::from(vec![4]);
        fork.header.hash = fork.header.inner.hash_slow();
        let state = Arc::new(WatchRpcState {
            request: request.clone(),
            second,
            second_receipt: Mutex::new(canonical_second.clone()),
            canonical_second,
            chain,
            fork,
            original_visible,
            processed: AtomicBool::new(false),
            account_nonce: AtomicU64::new(1),
            head: AtomicU64::new(107),
            finalized_height: AtomicU64::new(100),
            receipt: Mutex::new(Value::Null),
            canonical_receipt: serde_json::to_value(canonical)?,
            witness_fault: Mutex::new(RpcFault::None),
            witness_height: Mutex::new(None),
            pin_changed: AtomicBool::new(false),
            fault: Mutex::new(RpcFault::None),
            manager: Mutex::new(Address::from([5; 20])),
            calls: Mutex::new(Vec::new()),
        });
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    (accepted, witness) = async {
                        tokio::select! {
                            accepted = listener.accept() => (accepted, false),
                            accepted = witness_listener.accept() => (accepted, true),
                        }
                    } => {
                        let Ok((socket, _)) = accepted else { break };
                        let state = server_state.clone();
                        clients.spawn(async move {
                            let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { return };
                            while let Some(Ok(frame)) = socket.next().await {
                                if let WsMessage::Text(text) = frame {
                                    let request: Value = serde_json::from_str(&text).unwrap();
                                    let response = state.response(&request, witness);
                                    let silent = {
                                        let mut fault = state.fault.lock().unwrap();
                                        let silent = (*fault == RpcFault::SilentReceipt && request["method"] == "eth_getTransactionReceipt")
                                            || (*fault == RpcFault::SilentEvidence && request["method"] == "eth_getBlockReceipts");
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
        let beacon_state = state.clone();
        let beacon = LocalBeaconFixture::new(31337, state.chain[0].header.hash, Arc::new(move || {
            beacon_state.chain[beacon_state.finalized_height.load(Ordering::SeqCst) as usize].header.clone()
        })).await?;
        let api = api
            .with_beacon_finality(beacon.config.clone(), &beacon.directory.join("store"))
            .await?
            .with_data_witness(&witness_url, 31337, state.chain[0].header.hash)
            .await?;
        Ok(WatchRpc {
            api,
            state,
            server,
            _network: network,
            _beacon: beacon,
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
                assert!(rpc.api
                    .get_finalized_submission_snapshot(
                        &request.submission.as_ref().unwrap().raw_transaction,
                    )
                    .await?
                    .is_some());
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
    async fn retained_authentication_rejects_mutation_and_half_present_evidence() {
        let message = Message {
            nonce_be: [7; 32],
            source: [8; 32],
            destination: [5; 20],
            payload: vec![6; 104],
        };
        let signed = signed_message(&message).await;
        let original = evidence(&message, &signed, true);
        for case in [
            "transaction-proof",
            "receipt-proof",
            "raw",
            "index",
            "genesis",
            "chain",
            "policy",
            "version",
            "receipt",
            "missing-proof",
            "missing-checkpoint",
            "signature",
        ] {
            let mut candidate = original.clone();
            let mut saved = signed.clone();
            match case {
                "transaction-proof" => candidate
                    .inclusion
                    .as_mut()
                    .unwrap()
                    .transaction_proof
                    .clear(),
                "receipt-proof" => candidate.inclusion.as_mut().unwrap().receipt_proof.clear(),
                "raw" => candidate
                    .inclusion
                    .as_mut()
                    .unwrap()
                    .raw_transaction
                    .push(0),
                "index" => candidate.inclusion.as_mut().unwrap().index = 1,
                "genesis" => candidate.inclusion.as_mut().unwrap().genesis = B256::ZERO,
                "chain" => {
                    let FinalityCheckpoint::Legacy(checkpoint) = candidate.checkpoint.as_mut().unwrap() else { unreachable!() };
                    checkpoint.chain_id += 1;
                }
                "policy" | "version" => {
                    let mut value = serde_json::to_value(candidate.checkpoint.as_ref().unwrap()).unwrap();
                    value[if case == "policy" { "policy" } else { "schema_version" }] =
                        if case == "policy" { json!("single-provider") } else { json!(9) };
                    assert!(serde_json::from_value::<FinalityCheckpoint>(value).is_err(), "{case}");
                    continue;
                }
                "receipt" => candidate.receipt.transaction_index = Some(1),
                "missing-proof" => candidate.inclusion = None,
                "missing-checkpoint" => candidate.checkpoint = None,
                "signature" => saved.raw_transaction.push(0),
                _ => unreachable!(),
            }
            assert!(
                validate_completion(&message, &saved, &candidate).is_err(),
                "{case}"
            );
        }
        let mut legacy = original;
        legacy.inclusion = None;
        legacy.checkpoint = None;
        validate_completion(&message, &signed, &legacy).unwrap();
    }

    #[tokio::test]
    async fn rpc_state_answers_cannot_downgrade_approved_effect_policy() -> anyhow::Result<()> {
        let rpc = watch_rpc(true).await?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        *rpc.state.receipt.lock().unwrap() = rpc.state.canonical_receipt.clone();
        let request = &rpc.state.request;
        let signed = request.submission.as_ref().unwrap();
        let guard = rpc.api.reserve_submission().await;
        for fault in [RpcFault::Governance, RpcFault::Manager, RpcFault::Token, RpcFault::Processed, RpcFault::Nonce] {
            *rpc.state.fault.lock().unwrap() = fault;
            *rpc.state.witness_fault.lock().unwrap() = fault;
            let finalized = rpc.api.get_finalized_receipt(signed.hash).await?.unwrap();
            let completed = completion_evidence(
                &rpc.api, &request.message, signed, finalized, request.lane_policy.as_ref().unwrap(),
            ).await?;
            assert!(completed.token_delivery);
            validate_completion(&request.message, signed, &completed)?;
            assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
        }
        assert!(!rpc.state.calls.lock().unwrap().iter().any(|call| call["method"] == "eth_call"));
        *rpc.state.witness_fault.lock().unwrap() = RpcFault::WrongChain;
        assert!(rpc.api.get_finalized_receipt(signed.hash).await.is_err());
        *rpc.state.witness_fault.lock().unwrap() = RpcFault::None;
        rpc.state.pin_changed.store(true, Ordering::SeqCst);
        assert!(rpc.api.get_finalized_receipt(signed.hash).await.is_err());
        rpc.state.pin_changed.store(false, Ordering::SeqCst);
        drop(guard);
        Ok(())
    }

    #[tokio::test]
    async fn execution_finalized_tags_cannot_override_signed_beacon_head() -> anyhow::Result<()> {
        let rpc = watch_rpc(true).await?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        *rpc.state.witness_height.lock().unwrap() = Some(99);
        *rpc.state.receipt.lock().unwrap() = rpc.state.canonical_receipt.clone();
        for fault in [RpcFault::MissingFinality, RpcFault::InvalidFinality, RpcFault::ConflictingFinality] {
            *rpc.state.fault.lock().unwrap() = fault;
            *rpc.state.witness_fault.lock().unwrap() = fault;
            let included = rpc.api.get_finalized_receipt(rpc.state.request.tx_hash).await?.unwrap();
            assert_eq!(included.checkpoint.schema_version(), 2);
            assert_eq!(included.checkpoint.common_header().number, 107);
            assert!(included.checkpoint.beacon().is_some());
        }
        Ok(())
    }

    #[tokio::test]
    async fn missing_or_changed_policy_holds_before_receipt_lookup() -> anyhow::Result<()> {
        let rpc = watch_rpc(true).await?;
        for case in ["missing", "chain", "genesis", "queue", "source", "destination", "downgrade"] {
            let mut request = rpc.state.request.clone();
            if case == "missing" {
                request.lane_policy = None;
            } else {
                let policy = request.lane_policy.as_mut().unwrap();
                match case {
                    "chain" => policy.chain_id += 1,
                    "genesis" => policy.genesis_hash = B256::repeat_byte(99),
                    "queue" => policy.queue = Address::repeat_byte(99),
                    "source" => policy.source_manager = B256::repeat_byte(99),
                    "destination" => policy.destination = Address::repeat_byte(99),
                    "downgrade" => policy.token_delivery_required = false,
                    _ => unreachable!(),
                }
            }
            rpc.state.calls.lock().unwrap().clear();
            let (_, outcome) = watch_tx(rpc.api.clone(), request, 8).await;
            assert!(matches!(outcome, SubmissionOutcome::NeedsReconciliation { nonce_disposition: None, .. }), "{case}");
            assert!(!rpc.state.calls.lock().unwrap().iter().any(|call| call["method"] == "eth_getTransactionReceipt"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn malicious_rpc_downgrade_cannot_complete_token_without_bridged() -> anyhow::Result<()> {
        let rpc = watch_rpc_with_policy(true, false).await?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        *rpc.state.receipt.lock().unwrap() = rpc.state.canonical_receipt.clone();
        *rpc.state.manager.lock().unwrap() = Address::ZERO;
        for fault in [RpcFault::Governance, RpcFault::Manager, RpcFault::Token] {
            *rpc.state.fault.lock().unwrap() = fault;
            *rpc.state.witness_fault.lock().unwrap() = fault;
            let (_, outcome) = watch_tx(rpc.api.clone(), rpc.state.request.clone(), 8).await;
            assert!(matches!(outcome, SubmissionOutcome::NeedsReconciliation {
                diagnostic, nonce_disposition: None,
            } if diagnostic.contains("lacks exactly one")));
        }
        assert!(!rpc.state.calls.lock().unwrap().iter().any(|call| call["method"] == "eth_call"));
        Ok(())
    }

    #[tokio::test]
    async fn forged_rpc_receipt_never_authorizes_original_completion() -> anyhow::Result<()> {
        let rpc = watch_rpc(true).await?;
        let request = rpc.state.request.clone();
        let signed = request.submission.as_ref().unwrap();
        let mut forged = rpc.state.canonical_receipt.clone();
        // The locator looks plausible, but its cumulative gas is not in the receipt trie.
        forged["cumulativeGasUsed"] = json!("0x9999");
        *rpc.state.receipt.lock().unwrap() = forged;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        let guard = rpc.api.reserve_submission().await;
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            watch_tx(rpc.api.clone(), request.clone(), 8),
        )
        .await;
        assert!(!matches!(
            result,
            Ok((
                _,
                SubmissionOutcome::Completed(_) | SubmissionOutcome::FinalizedRevert(_)
            ))
        ));
        assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
        assert_eq!(
            request.submission.as_ref().unwrap().raw_transaction,
            signed.raw_transaction
        );
        drop(guard);
        Ok(())
    }

    #[tokio::test]
    async fn original_receipt_requires_finalized_confirmations_and_token_effect(
    ) -> anyhow::Result<()> {
        for valid in [true, false] {
            let rpc = watch_rpc_with_policy(true, valid).await?;
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
        let rpc = watch_rpc_with_policy(true, false).await?;
        *rpc.state.manager.lock().unwrap() = Address::ZERO;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        let mut request = rpc.state.request.clone();
        request.lane_policy = Some(lane_policy(
            &request.message, request.submission.as_ref().unwrap(), rpc.state.chain[0].header.hash, false,
        ));
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
            let result = tokio::time::timeout(
                Duration::from_millis(250),
                watch_tx(rpc.api.clone(), request.clone(), 8),
            )
            .await;
            if case == "valid" {
                assert!(!expect_completed(result?.1)?.token_delivery);
            } else {
                assert!(
                    !matches!(
                        result,
                        Ok((
                            _,
                            SubmissionOutcome::Completed(_) | SubmissionOutcome::FinalizedRevert(_)
                        ))
                    ),
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
            inclusion: failure.inclusion.clone().unwrap(),
            checkpoint: failure.checkpoint.clone().unwrap(),
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
            rpc.state.finalized_height.store(107, Ordering::SeqCst);
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
            assert!(snapshot.pinned_nonce_consumed());
            assert_eq!(snapshot.block_hash(), rpc.state.chain[107].header.hash);
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
    async fn saved_nonce_disposition_reauthenticates_original_pin_and_account_proof(
    ) -> anyhow::Result<()> {
        let rpc = watch_rpc(false).await?;
        rpc.state.finalized_height.store(101, Ordering::SeqCst);
        let raw = &rpc.state.request.submission.as_ref().unwrap().raw_transaction;
        let saved = rpc.api.get_finalized_submission_snapshot(raw).await?.unwrap();
        let saved: FinalizedSubmissionSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        let qualified = rpc.api.verify_finalized_submission_snapshot(raw, &saved).await?;
        assert_eq!(qualified.block_hash(), saved.block_hash());
        assert!(qualified.pinned_nonce_consumed());
        assert_eq!(qualified.beacon().unwrap().checkpoint.schema_version(), 2);
        for processed in [false, true] {
            let historical: FinalizedSubmissionSnapshot = serde_json::from_value(json!({
                "block_number": saved.block_number(),
                "block_hash": saved.block_hash(),
                "pinned_nonce_consumed": true,
                "message_processed": processed,
            }))?;
            let qualified = rpc.api.verify_finalized_submission_snapshot(raw, &historical).await?;
            assert_eq!(qualified.block_hash(), saved.block_hash());
            assert!(qualified.beacon().is_some());
            assert!(historical.legacy().is_some(), "qualification must not relabel historical evidence");
        }
        let mut damaged = saved.clone();
        let FinalizedSubmissionSnapshot::Beacon(proven) = &mut damaged else {
            panic!("live nonce evidence must use schema 2");
        };
        proven.account_proof.account_proof.clear();
        assert!(rpc.api.verify_finalized_submission_snapshot(raw, &damaged).await.is_err());
        *rpc.state.fault.lock().unwrap() = RpcFault::Nonce;
        assert!(rpc.api.get_finalized_submission_snapshot(raw).await.is_err(),
            "RPC nonce claim cannot override the account proof");
        *rpc.state.fault.lock().unwrap() = RpcFault::None;
        for (field, value) in [
            ("pinned_nonce_consumed", json!(false)),
            ("account_nonce", json!(0)),
            ("block_hash", json!(B256::repeat_byte(22))),
        ] {
            let mut value_json = serde_json::to_value(&saved)?;
            value_json[field] = value;
            if let Ok(changed) = serde_json::from_value::<FinalizedSubmissionSnapshot>(value_json) {
                assert!(rpc.api.verify_finalized_submission_snapshot(raw, &changed).await.is_err(), "{field}");
            }
        }
        *rpc.state.witness_fault.lock().unwrap() = RpcFault::WrongChain;
        assert!(rpc.api.verify_finalized_submission_snapshot(raw, &saved).await.is_err());
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
                request.submission.clone(),
                request.lane_policy.clone()
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
            request.submission.clone(),
            request.lane_policy.clone()
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
            request.submission.clone(),
            request.lane_policy.clone()
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
            request.submission.clone(),
            request.lane_policy.clone()
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
            request.submission,
            request.lane_policy
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), io.recv_message())
                .await
                .is_err()
        );
        assert_eq!(failed.get(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_journal_upgrade_authenticates_every_original_before_save_or_ack(
    ) -> anyhow::Result<()> {
        use crate::message_relayer::{
            common::{AuthoritySetId, GearBlockNumber, MessageInBlock},
            gear_to_eth::{
                storage::{JSONStorage, OutboundLaneIdentity, Storage},
                tx_manager::{Transaction, TransactionManager, TxStatus},
            },
        };
        use primitive_types::{H160, H256};
        let rpc = watch_rpc(true).await?;
        rpc.state.finalized_height.store(107, Ordering::SeqCst);
        *rpc.state.receipt.lock().unwrap() = rpc.state.canonical_receipt.clone();
        let directory = ScratchDirectory(
            std::env::temp_dir().join(format!("phase-one-journal-{}", Uuid::new_v4())),
        );
        let storage = Arc::new(JSONStorage::new(&directory.0));
        let sender = rpc.state.request.submission.as_ref().unwrap().sender;
        storage
            .bind_outbound_lane(OutboundLaneIdentity {
                destination_chain_id: 31337,
                destination_genesis_hash: H256(rpc.state.chain[0].header.hash.0),
                message_queue_address: H160::repeat_byte(2),
                bridging_payment_address: None,
                fee_exempt_sources: Default::default(),
                sender_address: H160::from_slice(sender.as_slice()),
            })
            .await?;
        storage.bind_event_chain(H256::repeat_byte(9), 42).await?;
        rpc.api
            .enable_finality_archive(
                &directory.0.join("ethereum-finality"),
                rpc.state.chain[0].header.hash,
            )
            .await?;
        let manager = TransactionManager::new(storage.clone());
        let mut originals = Vec::new();
        for (index, request) in [&rpc.state.request, &rpc.state.second]
            .into_iter()
            .enumerate()
        {
            let signed = request.submission.as_ref().unwrap();
            let finalized = rpc.api.get_finalized_receipt(signed.hash).await?.unwrap();
            let mut completion =
                completion_evidence(&rpc.api, &request.message, signed, finalized, request.lane_policy.as_ref().unwrap()).await?;
            completion.inclusion = None;
            completion.checkpoint = None;
            // Historical RPC classification may have downgraded an actual token delivery.
            completion.token_delivery = false;
            let message = MessageInBlock {
                message: request.message.clone(),
                block: GearBlockNumber(42),
                block_hash: H256::repeat_byte(99),
                authority_set_id: AuthoritySetId(5),
            };
            let mut tx = Transaction::new(message, TxStatus::Completed);
            tx.uuid = Uuid::from_u128(index as u128 + 1);
            tx.journal_version = 4;
            tx.ethereum_submission = Some(signed.clone());
            tx.ethereum_tx_attempts.push(signed.hash);
            tx.ethereum_tx_hash = Some(signed.hash);
            tx.completion = Some(completion);
            originals.push(tx.clone());
            manager.add_transaction(tx).await;
        }
        use sha2::Digest;
        let lanes = [&rpc.state.request, &rpc.state.second].into_iter().map(|request| {
            let mut lane = serde_json::to_value(request.lane_policy.as_ref().unwrap()).unwrap();
            lane.as_object_mut().unwrap().remove("approvalSha256");
            lane["originalHash"] = json!(request.tx_hash);
            lane
        }).collect::<Vec<_>>();
        let approval_bytes = serde_json::to_vec(&json!({"schemaVersion": 1, "lanes": lanes}))?;
        let approvals_directory = directory.0.join("approvals");
        std::fs::create_dir_all(&approvals_directory)?;
        let approval_path = approvals_directory.join("historical-lanes.json");
        std::fs::write(&approval_path, &approval_bytes)?;
        let approvals = crate::lane_policy::LaneApprovals::load(
            &approval_path, B256::from_slice(&sha2::Sha256::digest(&approval_bytes)),
        )?;
        let messages: Vec<_> = originals.iter().map(|tx| tx.message.clone()).collect();
        storage
            .record_queued_block(42, H256::repeat_byte(99), &messages)
            .await?;
        manager.update_storage().await?;
        let mut before = BTreeMap::new();
        for name in originals
            .iter()
            .flat_map(|tx| {
                [
                    tx.uuid.to_string(),
                    format!(
                        "completed_nonce_{}.json",
                        hex::encode(tx.message.message.nonce_be)
                    ),
                ]
            })
            .chain([
                "gear_events.json".to_owned(),
                "transaction_status.json".to_owned(),
            ])
        {
            before.insert(name.clone(), std::fs::read(directory.0.join(name))?);
        }
        assert_eq!(storage.queued_observations().await?.len(), 2);
        rpc.state.second_receipt.lock().unwrap()["cumulativeGasUsed"] = json!("0x9999");
        let restored = TransactionManager::new(storage.clone()).with_lane_approvals(
            rpc.api.clone(), approvals.clone(),
        );
        restored.load_from_storage().await?;
        assert!(restored.verify_completed(&rpc.api).await.is_err());
        for (name, bytes) in &before {
            assert_eq!(
                &std::fs::read(directory.0.join(name))?,
                bytes,
                "{name} changed before all originals passed"
            );
        }
        assert_eq!(storage.queued_observations().await?.len(), 2);
        *rpc.state.second_receipt.lock().unwrap() = rpc.state.canonical_second.clone();
        restored.verify_completed(&rpc.api).await?;
        assert!(storage.queued_observations().await?.is_empty());
        let mut upgraded = BTreeMap::new();
        for original in &originals {
            let bytes = std::fs::read(directory.0.join(original.uuid.to_string()))?;
            let saved: Transaction = serde_json::from_slice(&bytes)?;
            assert_eq!(saved.journal_version, 6);
            let completion = saved.completion.as_ref().unwrap();
            assert!(completion.inclusion.is_some() && completion.checkpoint.is_some());
            assert!(saved.ethereum_submission == original.ethereum_submission);
            assert_eq!(saved.historical_evidence.as_ref(), Some(&serde_json::to_value(original)?));
            assert_eq!(completion.checkpoint.as_ref().unwrap().schema_version(), 2);
            assert_eq!(
                completion.receipt,
                original.completion.as_ref().unwrap().receipt
            );
            assert!(completion.token_delivery, "approved token policy must replace obsolete RPC classification");
            assert_eq!(
                saved.historical_evidence.as_ref().unwrap()["completion"]["token_delivery"],
                json!(false),
                "historical classification must remain immutable",
            );
            upgraded.insert(original.uuid, bytes);
        }
        let restarted = TransactionManager::new(storage.clone()).with_lane_approvals(
            rpc.api.clone(), approvals,
        );
        restarted.load_from_storage().await?;
        *rpc.state.witness_fault.lock().unwrap() = RpcFault::WrongChain;
        assert!(restarted
            .verify_completed(&rpc.api.reconnect().await?)
            .await
            .is_err());
        for (uuid, bytes) in &upgraded {
            assert_eq!(&std::fs::read(directory.0.join(uuid.to_string()))?, bytes);
        }
        *rpc.state.witness_fault.lock().unwrap() = RpcFault::None;
        restarted.verify_completed(&rpc.api).await?;
        for (uuid, bytes) in &upgraded {
            assert_eq!(&std::fs::read(directory.0.join(uuid.to_string()))?, bytes);
        }
        Ok(())
    }

    struct ForwardRpc {
        url: String,
        fault: Arc<Mutex<RpcFault>>,
        server: tokio::task::JoinHandle<()>,
    }
    impl Drop for ForwardRpc {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    // Two owned loopback routes exercise disagreement, not production independence.
    async fn forward_rpc(upstream: &str) -> anyhow::Result<ForwardRpc> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}", listener.local_addr()?);
        let upstream = upstream.to_owned();
        let fault = Arc::new(Mutex::new(RpcFault::None));
        let server_fault = fault.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let upstream = upstream.clone();
                        let fault = server_fault.clone();
                        clients.spawn(async move {
                            let mut downstream = tokio_tungstenite::accept_async(socket).await.unwrap();
                            let socket = tokio::net::TcpStream::connect(upstream.trim_start_matches("ws://")).await.unwrap();
                            let (mut upstream, _) = tokio_tungstenite::client_async(upstream, socket).await.unwrap();
                            while let Some(Ok(WsMessage::Text(text))) = downstream.next().await {
                                let request: Value = serde_json::from_str(&text).unwrap();
                                if upstream.send(WsMessage::Text(text)).await.is_err() { break; }
                                let response = loop {
                                    match upstream.next().await {
                                        Some(Ok(WsMessage::Text(text))) => {
                                            let response: Value = serde_json::from_str(&text).unwrap();
                                            if response["id"] == request["id"] { break Some(response); }
                                        }
                                        Some(Ok(_)) => continue,
                                        _ => break None,
                                    }
                                };
                                let Some(mut response) = response else { break };
                                let active = *fault.lock().unwrap();
                                if active == RpcFault::Receipt && request["method"] == "eth_getTransactionReceipt"
                                    && response["result"].is_object() {
                                    let old = response["result"]["status"].clone();
                                    response["result"]["status"] = json!(if old == "0x1" { "0x0" } else { "0x1" });
                                }
                                if active == RpcFault::ConflictingFinality && request["method"] == "eth_getBlockByNumber"
                                    && request["params"][0] != "0x0" && response["result"].is_object() {
                                    let mut block: Block = serde_json::from_value(response["result"].clone()).unwrap();
                                    block.header.inner.extra_data = Bytes::from_static(b"private fork");
                                    block.header.hash = block.header.inner.hash_slow();
                                    response["result"] = json!(block);
                                }
                                if active == RpcFault::Manager && request["method"] == "eth_call" {
                                    let data = request["params"][0].get("input")
                                        .or_else(|| request["params"][0].get("data"));
                                    if data == Some(&json!(format!("0x{}", hex::encode(QueueGovernance::erc20ManagerCall::SELECTOR)))) {
                                        response["result"] = json!(format!("0x{:064x}", 0));
                                    }
                                }
                                if downstream.send(WsMessage::Text(response.to_string().into())).await.is_err() { break; }
                            }
                        });
                    }
                    joined = clients.join_next(), if !clients.is_empty() => {
                        if let Some(Err(error)) = joined { panic!("loopback forwarding failed: {error}"); }
                    }
                }
            }
        });
        Ok(ForwardRpc { url, fault, server })
    }

    async fn assert_forwarded_hold(
        api: &EthApi,
        request: &TrackedRequest,
        route: &ForwardRpc,
        faults: &[RpcFault],
    ) -> anyhow::Result<()> {
        for fault in faults {
            *route.fault.lock().unwrap() = *fault;
            let before = request.submission.as_ref().unwrap().raw_transaction.clone();
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                watch_tx(api.clone(), request.clone(), 8),
            )
            .await;
            assert!(!matches!(
                result,
                Ok((
                    _,
                    SubmissionOutcome::Completed(_) | SubmissionOutcome::FinalizedRevert(_)
                ))
            ));
            assert_eq!(request.submission.as_ref().unwrap().raw_transaction, before);
            assert!(futures::poll!(Box::pin(api.reserve_submission())).is_pending());
            *route.fault.lock().unwrap() = RpcFault::None;
        }
        Ok(())
    }

    struct ScratchDirectory(std::path::PathBuf);
    impl Drop for ScratchDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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
                    "31337",
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
        let transport = api.ok_or_else(|| anyhow::anyhow!("disposable Anvil failed to start"))?;
        let provider = transport.raw_provider();
        let primary = forward_rpc(&url).await?;
        let witness = forward_rpc(&url).await?;
        let genesis = provider
            .get_block_by_number(alloy::rpc::types::BlockNumberOrTag::Number(0))
            .await?
            .ok_or_else(|| anyhow::anyhow!("Anvil genesis missing"))?
            .header
            .hash;
        let beacon = LocalBeaconFixture::from_provider(
            provider.root().clone(), alloy::rpc::types::BlockNumberOrTag::Finalized,
        ).await?;
        let api = EthApi::new(&primary.url, &format!("{queue:#x}"), Some(&key), None, None)
            .await?
            .with_beacon_finality(beacon.config.clone(), &beacon.directory.join("store"))
            .await?
            .with_data_witness(&witness.url, 31337, genesis)
            .await?;
        let directory = ScratchDirectory(
            std::env::temp_dir().join(format!("phase-one-anvil-{}", Uuid::new_v4())),
        );
        api.enable_finality_archive(&directory.0.join("ethereum-finality"), genesis)
            .await?;
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
            lane_policy: Some(lane_policy(&message, &signed, genesis, false)),
        };
        let ownership = api.reserve_submission().await;
        assert_forwarded_hold(
            &api,
            &request,
            &primary,
            &[
                RpcFault::Receipt,
                RpcFault::ConflictingFinality,
            ],
        )
        .await?;
        let (_, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            watch_tx(api.clone(), request.clone(), 8),
        )
        .await?;
        let evidence = expect_completed(outcome)?;
        assert!(!evidence.token_delivery);
        let saved_path = directory.0.join("completion.json");
        let saved = serde_json::to_vec(&evidence)?;
        std::fs::write(&saved_path, &saved)?;
        let restored: CompletionEvidence = serde_json::from_slice(&std::fs::read(&saved_path)?)?;
        validate_completion(&message, &signed, &restored)?;
        let restarted_api = api.reconnect().await?;
        assert_forwarded_hold(
            &restarted_api,
            &request,
            &primary,
            &[
                RpcFault::Receipt,
                RpcFault::ConflictingFinality,
            ],
        )
        .await?;
        assert_eq!(std::fs::read(&saved_path)?, saved);
        let (_, restarted) =
            tokio::time::timeout(Duration::from_secs(10), watch_tx(restarted_api, request, 8))
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
            submission: Some(replay.clone()),
            lane_policy: Some(lane_policy(&message, &replay, genesis, false)),
        };
        assert_forwarded_hold(
            &api,
            &request,
            &primary,
            &[RpcFault::Receipt, RpcFault::ConflictingFinality],
        )
        .await?;
        let (_, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            watch_tx(api.clone(), request.clone(), 8),
        )
        .await?;
        let SubmissionOutcome::FinalizedRevert(failure) = outcome else {
            anyhow::bail!("real replay must qualify as a reverted original");
        };
        let failed_path = directory.0.join("failure.json");
        let failed_bytes = serde_json::to_vec(&failure)?;
        std::fs::write(&failed_path, &failed_bytes)?;
        let failure: FinalizedFailureEvidence =
            serde_json::from_slice(&std::fs::read(&failed_path)?)?;
        validate_failure(&message, &replay, &failure)?;
        let restarted = api.reconnect().await?;
        assert_forwarded_hold(
            &restarted,
            &request,
            &primary,
            &[RpcFault::Receipt, RpcFault::ConflictingFinality],
        )
        .await?;
        assert_eq!(std::fs::read(&failed_path)?, failed_bytes);
        let (_, outcome) =
            tokio::time::timeout(Duration::from_secs(10), watch_tx(restarted, request, 8)).await?;
        assert!(matches!(outcome, SubmissionOutcome::FinalizedRevert(_)));
        assert!(futures::poll!(Box::pin(api.reserve_submission())).is_pending());
        drop(ownership);
        println!("signed local Beacon Anvil: forged receipt/header HOLD; original success and replay-revert retained and revalidated after restart");
        let absent = signed_message_with_nonce(&message, 3).await;
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: absent.hash,
            lane_policy: Some(lane_policy(&message, &absent, genesis, false)),
            message,
            submission: Some(absent),
        };
        let absent = tokio::time::timeout(Duration::from_secs(1), watch_tx(api, request, 8)).await;
        assert!(absent.is_err());
        Ok(())
    }
}
