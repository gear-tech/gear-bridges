use alloy_primitives::FixedBytes;
use eth_events_electra_client::EthToVaraEvent;
use futures::executor::block_on;
use gclient::GearApi;
use gear_common::{api_provider::ApiProviderConnection, UNITS};
use gsdk::{
    ext::subxt::{config::polkadot::PolkadotExtrinsicParamsBuilder, tx::SubmittableTransaction},
    gear::{self, runtime_types::gear_common::event::MessageEntry},
};
use historical_proxy_client::historical_proxy::io::Redirect;
use primitive_types::{H160, H256};
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    IntCounter, IntGauge,
};
use sails_rs::{
    calls::*,
    gclient::calls::{GClientRemoting, QueryExtGClient},
    Decode, Encode,
};
use serde::{Deserialize, Serialize};
use std::{ops::Deref, str::FromStr};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;
use vft_manager_client::{
    traits::VftManager as _,
    vft_manager::io::{ReconcileReceipt, SubmitReceipt},
    VftManager,
};

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

    pub fn prepare_message(
        &mut self,
        tx_uuid: Uuid,
        tx_hash: FixedBytes<32>,
        payload: EthToVaraEvent,
    ) -> bool {
        self.requests_channel
            .send(Request::PrepareMessage {
                tx_uuid,
                tx_hash,
                payload: Box::new(payload),
            })
            .inspect_err(|err| log::error!("Message sender failed: {err:?}"))
            .is_ok()
    }

    pub fn submit_prepared(
        &mut self,
        tx_uuid: Uuid,
        tx_hash: FixedBytes<32>,
        payload: EthToVaraEvent,
        signed_submission: SignedSubmission,
        settlement_pin: Option<FinalizedReceiptObservation>,
    ) -> bool {
        self.requests_channel
            .send(Request::SubmitPrepared {
                tx_uuid,
                tx_hash,
                prepared: Box::new((payload, signed_submission)),
                settlement_pin,
            })
            .inspect_err(|err| log::error!("Message sender failed: {err:?}"))
            .is_ok()
    }

    pub fn reconcile_receipt(&mut self, tx_uuid: Uuid, receipt_key: (u64, u64)) -> bool {
        self.requests_channel
            .send(Request::ReceiptStatus {
                tx_uuid,
                receipt_key,
            })
            .inspect_err(|err| log::error!("Message sender failed: {err:?}"))
            .is_ok()
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses_channel.recv().await
    }
}

#[derive(Clone, Debug)]
pub enum Request {
    PrepareMessage {
        payload: Box<EthToVaraEvent>,
        tx_hash: FixedBytes<32>,
        tx_uuid: Uuid,
    },
    SubmitPrepared {
        prepared: Box<(EthToVaraEvent, SignedSubmission)>,
        settlement_pin: Option<FinalizedReceiptObservation>,
        tx_hash: FixedBytes<32>,
        tx_uuid: Uuid,
    },
    ReceiptStatus {
        tx_uuid: Uuid,
        receipt_key: (u64, u64),
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinalizedReceiptObservation {
    pub finalized_block_number: Option<u32>,
    pub finalized_block_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SignedSubmission {
    pub chain_genesis_hash: String,
    pub manager: String,
    pub historical_proxy: String,
    pub sender: String,
    pub receipt_key: (u64, u64),
    pub payload_hash: String,
    pub nonce: u64,
    pub extrinsic_hash: String,
    pub raw_extrinsic: Vec<u8>,
    pub prepared_finalized_number: u32,
    pub prepared_finalized_hash: String,
    pub scanned_finalized_number: u32,
    pub scanned_finalized_hash: String,
    pub reply_scanned_finalized_number: u32,
    pub reply_scanned_finalized_hash: String,
    pub inclusion_block_number: Option<u32>,
    pub inclusion_block_hash: Option<String>,
    pub message_id: Option<String>,
    #[serde(default)]
    pub finalized_dispatch_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_reply: Option<FinalizedReplyEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_reconciliation: Option<Box<SignedSubmission>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_reconciliation_attempts: Vec<SignedSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_nonce_consumed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_reply_error: Option<String>,
}

impl SignedSubmission {
    pub fn same_request(&self, other: &Self) -> bool {
        self.chain_genesis_hash == other.chain_genesis_hash
            && self.manager == other.manager
            && self.historical_proxy == other.historical_proxy
            && self.sender == other.sender
            && self.receipt_key == other.receipt_key
            && self.payload_hash == other.payload_hash
            && self.nonce == other.nonce
            && self.extrinsic_hash == other.extrinsic_hash
            && self.raw_extrinsic == other.raw_extrinsic
            && self.prepared_finalized_number == other.prepared_finalized_number
            && self.prepared_finalized_hash == other.prepared_finalized_hash
    }

    pub fn validate_native_reconciliation(&self) -> anyhow::Result<()> {
        let call = ReconcileReceipt::encode_call(self.receipt_key.0, self.receipt_key.1);
        let mut nonce = self.nonce;
        for (index, continuation) in self
            .native_reconciliation_attempts
            .iter()
            .chain(self.native_reconciliation.as_deref())
            .enumerate()
        {
            anyhow::ensure!(
                self.has_finalized_dispatch_reply()
                    && continuation.native_reconciliation.is_none()
                    && continuation.native_reconciliation_attempts.is_empty()
                    && self.finalized_reply.as_ref().is_some_and(|reply| reply
                        .observation
                        .finalized_block_number
                        .is_some_and(|number| continuation.prepared_finalized_number >= number))
                    && continuation.chain_genesis_hash == self.chain_genesis_hash
                    && continuation.manager == self.manager
                    && continuation.historical_proxy == self.historical_proxy
                    && continuation.sender == self.sender
                    && continuation.receipt_key == self.receipt_key
                    && continuation.payload_hash
                        == format!("{:#x}", alloy_primitives::keccak256(&call))
                    && continuation.nonce > nonce
                    && !continuation.raw_extrinsic.is_empty()
                    && continuation.raw_hash_matches(),
                "HOLD: original native reconciliation identity or exact signed bytes changed"
            );
            if index < self.native_reconciliation_attempts.len() {
                anyhow::ensure!(
                    continuation.has_terminal_reconciliation(),
                    "HOLD: archived reconciliation lacks definite canonical terminal evidence"
                );
            }
            nonce = continuation.nonce;
        }
        Ok(())
    }

    pub fn has_terminal_reconciliation(&self) -> bool {
        (self.inclusion_block_number.is_none()
            && self.message_id.is_none()
            && self.finalized_dispatch_error.is_none()
            && self.finalized_reply.is_none()
            && self
                .finalized_nonce_consumed
                .is_some_and(|nonce| nonce > self.nonce)
            && self.scanned_finalized_number >= self.prepared_finalized_number)
            || (self.inclusion_block_number.is_some()
                && self.inclusion_block_hash.is_some()
                && self.message_id.is_none()
                && self.finalized_dispatch_error.is_some()
                && self.finalized_reply.is_none())
            || (self
                .inclusion_block_number
                .zip(
                    self.finalized_reply
                        .as_ref()
                        .and_then(|reply| reply.observation.finalized_block_number),
                )
                .is_some_and(|(included, replied)| replied >= included)
                && self.inclusion_block_hash.is_some()
                && self.message_id.is_some()
                && self.finalized_dispatch_error.is_none())
    }

    pub fn preserves_reconciliation(&self, previous: &Self) -> bool {
        self.native_reconciliation_attempts
            .starts_with(&previous.native_reconciliation_attempts)
            && previous
                .native_reconciliation
                .as_deref()
                .is_none_or(|original| {
                    self.native_reconciliation
                        .as_deref()
                        .filter(|current| original.same_request(current))
                        .or_else(|| {
                            self.native_reconciliation_attempts
                                .get(previous.native_reconciliation_attempts.len())
                        })
                        .is_some_and(|current| {
                            original.same_request(current)
                                && original.inclusion_block_number.is_none_or(|number| {
                                    current.inclusion_block_number == Some(number)
                                })
                                && original.inclusion_block_hash.as_ref().is_none_or(|hash| {
                                    current.inclusion_block_hash.as_ref() == Some(hash)
                                })
                                && original
                                    .message_id
                                    .as_ref()
                                    .is_none_or(|id| current.message_id.as_ref() == Some(id))
                                && original
                                    .finalized_dispatch_error
                                    .as_ref()
                                    .is_none_or(|error| {
                                        current.finalized_dispatch_error.as_ref() == Some(error)
                                    })
                                && original.finalized_reply.as_ref().is_none_or(|reply| {
                                    current.finalized_reply.as_ref() == Some(reply)
                                })
                                && original.finalized_reply_error.as_ref().is_none_or(|error| {
                                    current.finalized_reply_error.as_ref() == Some(error)
                                })
                                && original.finalized_nonce_consumed.is_none_or(|nonce| {
                                    current.finalized_nonce_consumed == Some(nonce)
                                })
                        })
                })
    }

    pub(super) fn has_finalized_dispatch_reply(&self) -> bool {
        self.inclusion_block_number
            .zip(
                self.finalized_reply
                    .as_ref()
                    .and_then(|reply| reply.observation.finalized_block_number),
            )
            .is_some_and(|(inclusion, reply)| reply >= inclusion)
            && self.inclusion_block_hash.is_some()
            && self.message_id.is_some()
            && self.finalized_dispatch_error.is_none()
            && self.finalized_reply_error.is_none()
            && self.finalized_reply.as_ref().is_some_and(|reply| {
                !reply.payload.is_empty() && !reply.observation.finalized_block_hash.is_empty()
            })
    }
    pub(super) fn validate_settlement_order(
        &self,
        pin: &FinalizedReceiptObservation,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.has_finalized_dispatch_reply(),
            "HOLD: saved completion lacks original finalized proof reply"
        );
        let number = pin
            .finalized_block_number
            .ok_or_else(|| anyhow::anyhow!("HOLD: saved settlement has no height"))?;
        for original in std::iter::once(self).chain(self.native_reconciliation.as_deref()) {
            anyhow::ensure!(
                original.has_finalized_dispatch_reply()
                    && original
                        .finalized_reply
                        .as_ref()
                        .and_then(|reply| reply.observation.finalized_block_number)
                        .is_some_and(|reply| number >= reply),
                "HOLD: saved settlement pin predates an original finalized reply"
            );
        }
        for archived in &self.native_reconciliation_attempts {
            let terminal = archived
                .finalized_reply
                .as_ref()
                .and_then(|reply| reply.observation.finalized_block_number)
                .or(archived.inclusion_block_number)
                .unwrap_or(archived.scanned_finalized_number);
            anyhow::ensure!(
                number >= terminal,
                "HOLD: saved settlement predates an archived terminal continuation"
            );
        }
        Ok(())
    }

    pub(super) fn raw_hash_matches(&self) -> bool {
        use subxt::config::{substrate::BlakeTwo256, Hasher};
        H256::from_str(&self.extrinsic_hash).ok()
            == Some(H256::from(BlakeTwo256.hash(&self.raw_extrinsic).0))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinalizedReplyEvidence {
    pub payload: Vec<u8>,
    pub observation: FinalizedReceiptObservation,
}

#[derive(Clone, Debug)]
pub enum MessageStatus {
    Prepared,
    PrepareFailed(String),
    Success {
        receipt_observation: Option<FinalizedReceiptObservation>,
    },
    Failure(String),
    RetryableNoop {
        receipt_key: (u64, u64),
        diagnostic: String,
        receipt_observation: Option<FinalizedReceiptObservation>,
    },
    NeedsReconciliation {
        receipt_key: (u64, u64),
        diagnostic: String,
        receipt_observation: Option<FinalizedReceiptObservation>,
    },
}

#[derive(Debug)]
struct ReceiptQueryFailure {
    diagnostic: String,
    receipt_observation: Option<FinalizedReceiptObservation>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubmissionEvidence {
    pub observed_at_ms: u64,
    pub manager: H256,
    pub historical_proxy: H256,
    pub sender: String,
    pub manager_reply: Option<Vec<u8>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub tx_uuid: Uuid,
    pub status: MessageStatus,
    pub submission: Option<SubmissionEvidence>,
    pub signed_submission: Option<SignedSubmission>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_millis()
        .try_into()
        .expect("Unix milliseconds fit u64")
}

fn send_reconciliation_response(
    responses: &mut UnboundedSender<Response>,
    tx_uuid: Uuid,
    receipt_key: (u64, u64),
    diagnostic: String,
    submission: Option<SubmissionEvidence>,
    signed_submission: Option<SignedSubmission>,
) -> bool {
    let diagnostic = format!(
        "{diagnostic}; receipt {receipt_key:?} is held; signed transaction bytes remain unchanged"
    );
    log::warn!("{diagnostic}");
    responses
        .send(Response {
            tx_uuid,
            submission,
            signed_submission,
            status: MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic,
                receipt_observation: None,
            },
        })
        .is_ok()
}

impl std::fmt::Display for FinalizedReceiptObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.finalized_block_number {
            Some(number) => write!(
                formatter,
                "finalized Gear block {number} ({})",
                self.finalized_block_hash
            ),
            None => write!(
                formatter,
                "finalized Gear block {} (height unavailable)",
                self.finalized_block_hash
            ),
        }
    }
}

fn receipt_query_result(
    receipt_key: (u64, u64),
    result: Result<
        (
            vft_manager_client::ReceiptStatus,
            FinalizedReceiptObservation,
        ),
        ReceiptQueryFailure,
    >,
) -> MessageStatus {
    match result {
        // Pair-keyed legacy history cannot prove any original dispatch or per-log effect.
        // Even Processed must remain held until the original signed request is reconciled.
        Ok((status, receipt_observation)) => MessageStatus::NeedsReconciliation {
            receipt_key,
            diagnostic: format!(
                "Receipt {receipt_key:?} is {status:?} at {receipt_observation}; original dispatch/reply and every deposit remain unverified; it will not be resubmitted"
            ),
            receipt_observation: Some(receipt_observation),
        },
        Err(error) => {
            let diagnostic = match error.receipt_observation.as_ref() {
                Some(observation) => format!(
                    "Receipt status for {receipt_key:?} could not be confirmed at {observation}: {}; it will not be resubmitted",
                    error.diagnostic
                ),
                None => format!(
                    "Receipt status for {receipt_key:?} could not be confirmed at a pinned finalized Gear block: {}; it will not be resubmitted",
                    error.diagnostic
                ),
            };
            MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic,
                receipt_observation: error.receipt_observation,
            }
        }
    }
}

impl_metered_service!(
    struct Metrics {
        fee_payer_balance: IntGauge = IntGauge::new(
            "gear_message_sender_fee_payer_balance",
            "Balance of the fee payer account",
        ),

        total_gas_used: GenericCounter<AtomicU64> = GenericCounter::new(
            "gear_message_sender_total_gas_used",
            "Total gas used by gear message sender",
        ),
        min_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_min_gas_used",
            "Minimum gas used by gear message sender",
        ),
        max_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_max_gas_used",
            "Maximum gas used by gear message sender",
        ),
        last_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "gear_message_sender_last_gas_used",
            "Last gas used by gear message sender",
        ),
        total_submissions: IntCounter = IntCounter::new(
            "gear_message_sender_total_submissions",
            "Total number of messages sent to Gear",
        ),
    }
);

pub(super) fn decode_complete_reply<I: ActionIo>(reply: &[u8]) -> anyhow::Result<I::Reply> {
    let mut input = reply
        .strip_prefix(I::ROUTE)
        .ok_or_else(|| anyhow::anyhow!("Reply route does not match the original consumer"))?;
    let decoded = I::Reply::decode(&mut input)?;
    anyhow::ensure!(input.is_empty(), "Reply contains trailing SCALE bytes");
    Ok(decoded)
}

// Custom consumers explicitly return SCALE Result<T, E>. Their bodies are opaque;
// only the authenticated application's result is qualified, never token settlement.
fn custom_consumer_status(
    route: &[u8],
    reply: &[u8],
    receipt_key: (u64, u64),
    observation: FinalizedReceiptObservation,
) -> MessageStatus {
    let result = reply.strip_prefix(route).and_then(|body| body.first());
    if route != SubmitReceipt::ROUTE && result == Some(&0) {
        MessageStatus::Success {
            receipt_observation: Some(observation),
        }
    } else {
        MessageStatus::NeedsReconciliation {
            receipt_key,
            diagnostic: format!(
                "Custom consumer did not return an authenticated Result::Ok for its exact route; opaque reply: 0x{}",
                hex::encode(reply)
            ),
            receipt_observation: Some(observation),
        }
    }
}

fn validate_deposit_evidence(
    receipt_rlp: &[u8],
    receipt_key: (u64, u64),
    ethereum_manager: H160,
    manager: H256,
    proxy: H256,
    deposits: &[vft_manager_client::ReceiptDepositState],
) -> anyhow::Result<()> {
    use alloy::sol_types::SolEvent;
    use alloy_rlp::Decodable;
    use ethereum_client::abi::IERC20Manager::BridgingRequested;
    use sails_rs::ActorId;

    let mut input = receipt_rlp;
    let receipt = ethereum_common::utils::ReceiptEnvelope::decode(&mut input)?;
    anyhow::ensure!(
        input.is_empty() && receipt.is_success(),
        "Original receipt framing or execution status is invalid"
    );
    let receipt_hash = H256::from(alloy_primitives::keccak256(receipt_rlp).0);
    let mut expected = 0;
    for (log_index, log) in receipt.logs().iter().enumerate() {
        if H160::from(log.address.0 .0) != ethereum_manager
            || log.topics().first() != Some(&BridgingRequested::SIGNATURE_HASH)
        {
            continue;
        }
        let event = BridgingRequested::decode_raw_log_validate(log.topics(), &log.data.data)?;
        let row = deposits.get(expected).ok_or_else(|| {
            anyhow::anyhow!("Original receipt deposit {log_index} has no retained economic outcome")
        })?;
        let operation_id = H256::from(
            alloy_primitives::keccak256(
                (
                    b"vara/native-escrow/v1",
                    ActorId::from(manager.0),
                    ActorId::from(proxy.0),
                    ethereum_manager,
                    receipt_key.0,
                    receipt_key.1,
                    log_index as u64,
                    receipt_hash,
                )
                    .encode(),
            )
            .0,
        );
        anyhow::ensure!(
            row.log_index == log_index as u64
                && row.eth_token_id.0 == event.token.0 .0
                && row.sender.0 == event.from.0 .0
                && row.receiver == ActorId::from(event.to.0)
                && row.amount == sails_rs::U256::from_little_endian(event.amount.as_le_slice())
                && row.operation_id.0 == operation_id.0
                && !row.token_id.is_zero()
                && row.child.is_some()
                && (!row.native || matches!(row.supply, vft_manager_client::TokenSupply::Gear)),
            "Economic outcome differs from original receipt deposit {log_index}"
        );
        expected += 1;
    }
    anyhow::ensure!(
        expected != 0 && deposits.len() == expected,
        "Receipt deposit count differs from the complete original receipt"
    );
    Ok(())
}

fn validate_settlement_pin(
    pin: &FinalizedReceiptObservation,
    finalized: u32,
    canonical_hash: H256,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pin.finalized_block_number
            .is_some_and(|number| number <= finalized)
            && H256::from_str(&pin.finalized_block_hash).ok() == Some(canonical_hash),
        "HOLD: historical settlement pin is not canonical finalized history"
    );
    Ok(())
}

fn native_payout_resolved(
    row: &vft_manager_client::ReceiptDepositState,
    manager: H256,
    payout: &vft_vara_client::Redemption,
) -> bool {
    payout.from == sails_rs::ActorId::from(manager.0)
        && payout.to == row.receiver
        && payout.amount == row.amount
        && !payout.child.is_zero()
        && row.child.is_some_and(|child| child != payout.child)
        && match payout.status {
            vft_vara_client::PayoutStatus::Delivered => {
                payout.returned_value == 0
                    && matches!(
                        row.outcome,
                        vft_manager_client::ReceiptDepositOutcome::Settled
                            | vft_manager_client::ReceiptDepositOutcome::NativeQueued
                    )
            }
            vft_vara_client::PayoutStatus::Refunded => {
                sails_rs::U256::from(payout.returned_value) == row.amount
                    && matches!(
                        row.outcome,
                        vft_manager_client::ReceiptDepositOutcome::Refunded
                            | vft_manager_client::ReceiptDepositOutcome::NativeQueued
                    )
            }
            _ => false,
        }
}

pub struct MessageSender {
    pub receiver_address: H256,
    pub receiver_route: Vec<u8>,
    pub historical_proxy_address: H256,
    pub api_provider: ApiProviderConnection,
    pub suri: String,
    ethereum_manager_address: Option<H160>,

    metrics: Metrics,
    // The actual detached worker owns the signer fence until its last original call exits.
    _signer_ownership: Option<std::fs::File>,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MessageSender {
    pub fn new(
        receiver_address: H256,
        receiver_route: Vec<u8>,
        historical_proxy_address: H256,
        api_provider: ApiProviderConnection,
        suri: String,
        ethereum_manager_address: Option<H160>,
    ) -> Self {
        Self {
            receiver_address,
            ethereum_manager_address,
            receiver_route,
            historical_proxy_address,
            api_provider,
            suri,

            metrics: Metrics::new(),
            _signer_ownership: None,
        }
    }

    pub fn with_signer_ownership(mut self, ownership: std::fs::File) -> Self {
        self._signer_ownership = Some(ownership);
        self
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
        self.update_balance_metric(&gear_api).await?;

        while let Some(request) = requests.recv().await {
            let error = match self.process(responses, &gear_api, &request).await {
                Ok(true) => continue,
                Ok(false) => return Ok(()),
                Err(error) => error,
            };
            let diagnostic = format!("Inbound submission processing failed: {error:#}");
            let sent = match &request {
                Request::PrepareMessage {
                    tx_uuid, tx_hash, ..
                } => responses
                    .send(Response {
                        tx_uuid: *tx_uuid,
                        status: MessageStatus::PrepareFailed(format!(
                            "Could not prepare signed transaction for {tx_hash:?}: {diagnostic}"
                        )),
                        submission: None,
                        signed_submission: None,
                    })
                    .is_ok(),
                Request::SubmitPrepared {
                    tx_uuid,
                    prepared,
                    tx_hash,
                    ..
                } => {
                    let (payload, signed_submission) = prepared.as_ref();
                    send_reconciliation_response(
                        responses,
                        *tx_uuid,
                        (payload.proof_block.block.slot, payload.transaction_index),
                        format!("Exact signed transaction {tx_hash:?} is unresolved: {diagnostic}"),
                        Some(self.submission_evidence(&gear_api, Some(diagnostic.clone()), None)),
                        Some(signed_submission.clone()),
                    )
                }
                Request::ReceiptStatus {
                    tx_uuid,
                    receipt_key,
                } => send_reconciliation_response(
                    responses,
                    *tx_uuid,
                    *receipt_key,
                    format!("Receipt status query failed: {diagnostic}"),
                    None,
                    None,
                ),
            };
            if !sent {
                return Ok(());
            }
            return Err(error);
        }

        Ok(())
    }

    async fn process(
        &mut self,
        responses: &mut UnboundedSender<Response>,
        gear_api: &GearApi,
        request: &Request,
    ) -> anyhow::Result<bool> {
        if matches!(
            request,
            Request::PrepareMessage { .. } | Request::SubmitPrepared { .. }
        ) {
            self.update_balance_metric(gear_api).await?;
        }

        match request {
            Request::ReceiptStatus {
                tx_uuid,
                receipt_key,
            } => {
                let status = receipt_query_result(
                    *receipt_key,
                    self.query_receipt_status(gear_api, *receipt_key).await,
                );
                Ok(responses
                    .send(Response {
                        tx_uuid: *tx_uuid,
                        status,
                        submission: None,
                        signed_submission: None,
                    })
                    .is_ok())
            }
            Request::PrepareMessage {
                tx_uuid,
                payload,
                tx_hash,
            } => {
                let (status, signed_submission) =
                    match self.prepare_submission(gear_api, payload).await {
                        Ok(signed_submission) => (MessageStatus::Prepared, Some(signed_submission)),
                        Err(error) => (
                            MessageStatus::PrepareFailed(format!(
                                "Could not prepare signed submission for {tx_hash:?}: {error:#}"
                            )),
                            None,
                        ),
                    };
                Ok(responses
                    .send(Response {
                        tx_uuid: *tx_uuid,
                        status,
                        submission: None,
                        signed_submission,
                    })
                    .is_ok())
            }
            Request::SubmitPrepared {
                tx_uuid,
                prepared,
                tx_hash,
                settlement_pin,
            } => {
                let (payload, signed_submission) = prepared.as_ref();
                let mut signed_submission = signed_submission.clone();
                let receipt_key = (payload.proof_block.block.slot, payload.transaction_index);
                let (status, outcome) = match self
                    .reconcile_signed_submission(
                        gear_api,
                        payload,
                        &mut signed_submission,
                        settlement_pin.as_ref(),
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        let diagnostic = format!(
                            "Exact signed submission for {tx_hash:?} is unresolved: {error:#}"
                        );
                        (
                            MessageStatus::NeedsReconciliation {
                                receipt_key,
                                diagnostic: diagnostic.clone(),
                                receipt_observation: None,
                            },
                            Some(Err(diagnostic)),
                        )
                    }
                };
                let error = match &status {
                    MessageStatus::Success { .. } => None,
                    MessageStatus::Failure(error)
                    | MessageStatus::PrepareFailed(error)
                    | MessageStatus::NeedsReconciliation {
                        diagnostic: error, ..
                    }
                    | MessageStatus::RetryableNoop {
                        diagnostic: error, ..
                    } => {
                        log::warn!("{error}");
                        Some(error.as_str())
                    }
                    MessageStatus::Prepared => None,
                };
                let submission = outcome.map(|outcome| match outcome {
                    Ok(reply) => {
                        self.submission_evidence(gear_api, error.map(str::to_owned), Some(reply))
                    }
                    Err(error) => self.submission_evidence(gear_api, Some(error), None),
                });
                Ok(responses
                    .send(Response {
                        tx_uuid: *tx_uuid,
                        status,
                        submission,
                        signed_submission: Some(signed_submission),
                    })
                    .is_ok())
            }
        }
    }

    pub async fn prepare_submission(
        &self,
        gear_api: &GearApi,
        payload: &EthToVaraEvent,
    ) -> anyhow::Result<SignedSubmission> {
        if self.receiver_route == SubmitReceipt::ROUTE {
            super::cutover::ensure_receipt_domain(
                gear_api,
                self.receiver_address,
                payload.proof_block.block.slot,
            )
            .await?;
        }
        let proof = payload.encode();
        let payload_hash = format!("{:#x}", alloy_primitives::keccak256(&proof));
        let call_payload = Redirect::encode_call(
            payload.proof_block.block.slot,
            proof,
            self.receiver_address.0.into(),
            self.receiver_route.clone(),
        );
        self.prepare_call(
            gear_api,
            (payload.proof_block.block.slot, payload.transaction_index),
            self.historical_proxy_address,
            call_payload,
            payload_hash,
        )
        .await
    }

    pub async fn prepare_call(
        &self,
        gear_api: &GearApi,
        receipt_key: (u64, u64),
        destination: H256,
        call_payload: Vec<u8>,
        payload_hash: String,
    ) -> anyhow::Result<SignedSubmission> {
        let chain = self.api_provider.client();
        let signer: gsdk::signer::Signer = gear_api.clone().into();
        anyhow::ensure!(
            signer.account_id().to_string() == gear_api.account_id().to_string(),
            "GSDK and GClient derived different submission accounts"
        );
        let nonce = chain.api.tx().account_nonce(signer.account_id()).await?;
        let gas_limit = gear_api.block_gas_limit()? / 100 * 95;
        let call =
            gear::tx()
                .gear()
                .send_message(destination.0.into(), call_payload, gas_limit, 0, false);
        let signed = chain
            .api
            .tx()
            .create_signed(
                &call,
                signer.signer(),
                PolkadotExtrinsicParamsBuilder::new().nonce(nonce).build(),
            )
            .await?;
        let finalized_hash = chain.latest_finalized_block().await?;
        let finalized_number = chain.block_hash_to_number(finalized_hash).await?;
        let finalized_hash = format!("{finalized_hash:#x}");
        Ok(SignedSubmission {
            chain_genesis_hash: format!("{:#x}", chain.api.genesis_hash()),
            manager: format!("{:#x}", self.receiver_address),
            historical_proxy: format!("{:#x}", self.historical_proxy_address),
            sender: gear_api.account_id().to_string(),
            receipt_key,
            payload_hash,
            nonce,
            extrinsic_hash: format!("{:#x}", signed.hash()),
            raw_extrinsic: signed.encoded().to_vec(),
            prepared_finalized_number: finalized_number,
            prepared_finalized_hash: finalized_hash.clone(),
            scanned_finalized_number: finalized_number,
            scanned_finalized_hash: finalized_hash.clone(),
            reply_scanned_finalized_number: finalized_number,
            reply_scanned_finalized_hash: finalized_hash,
            inclusion_block_number: None,
            inclusion_block_hash: None,
            message_id: None,
            finalized_dispatch_error: None,
            finalized_reply: None,
            native_reconciliation: None,
            native_reconciliation_attempts: Vec::new(),
            finalized_nonce_consumed: None,
            finalized_reply_error: None,
        })
    }

    /// Requalify sealed original evidence without signing, broadcasting, or mutating a journal.
    /// The caller must bind the signed sender and runtime to the independently approved seal.
    pub async fn verify_saved_completion(
        &self,
        gear_api: &GearApi,
        payload: &EthToVaraEvent,
        signed: &SignedSubmission,
        pin: &FinalizedReceiptObservation,
    ) -> anyhow::Result<()> {
        let mut original = signed.clone();
        let (status, _) = self
            .reconcile_signed_submission(gear_api, payload, &mut original, Some(pin))
            .await?;
        anyhow::ensure!(
            matches!(status, MessageStatus::Success { .. }),
            "HOLD: original historical completion could not be fully authenticated: {status:?}"
        );
        Ok(())
    }

    pub async fn reconcile_signed_submission(
        &self,
        gear_api: &GearApi,
        payload: &EthToVaraEvent,
        signed: &mut SignedSubmission,
        settlement_pin: Option<&FinalizedReceiptObservation>,
    ) -> anyhow::Result<(MessageStatus, Option<Result<Vec<u8>, String>>)> {
        signed.validate_native_reconciliation()?;
        let receipt_key = (payload.proof_block.block.slot, payload.transaction_index);
        let chain = self.api_provider.client();
        anyhow::ensure!(
            signed.chain_genesis_hash == format!("{:#x}", chain.api.genesis_hash()),
            "Signed submission belongs to another Gear genesis"
        );
        anyhow::ensure!(
            signed.manager == format!("{:#x}", self.receiver_address)
                && signed.historical_proxy == format!("{:#x}", self.historical_proxy_address)
                && (settlement_pin.is_some() || signed.sender == gear_api.account_id().to_string())
                && signed.receipt_key == receipt_key
                && signed.payload_hash
                    == format!("{:#x}", alloy_primitives::keccak256(payload.encode())),
            "Signed submission identity does not match the original request"
        );
        anyhow::ensure!(
            signed.raw_hash_matches(),
            "Stored signed transaction bytes do not match their recorded hash"
        );

        for archived in &signed.native_reconciliation_attempts {
            self.authenticate_archived_continuation(archived).await?;
        }
        let original_call = Redirect::encode_call(
            receipt_key.0,
            payload.encode(),
            self.receiver_address.0.into(),
            self.receiver_route.clone(),
        );
        if !self
            .find_signed_extrinsic(signed, self.historical_proxy_address, &original_call)
            .await?
        {
            anyhow::ensure!(
                settlement_pin.is_none(),
                "HOLD: saved completion has no original finalized proof inclusion"
            );
            let scanned_hash = H256::from_str(&signed.scanned_finalized_hash)?;
            let account = chain
                .api
                .info_at(&signed.sender, Some(scanned_hash))
                .await?;
            if u64::from(account.nonce) > signed.nonce {
                return Ok((MessageStatus::RetryableNoop {
                    receipt_key,
                    diagnostic: format!(
                        "Signed transaction {} is absent through finalized block {} and its nonce {} was consumed",
                        signed.extrinsic_hash, signed.scanned_finalized_number, signed.nonce,
                    ),
                    receipt_observation: Some(FinalizedReceiptObservation {
                        finalized_block_number: Some(signed.scanned_finalized_number),
                        finalized_block_hash: signed.scanned_finalized_hash.clone(),
                    }),
                }, None));
            }
            let transaction = SubmittableTransaction::from_bytes(
                chain.api.deref().clone(),
                signed.raw_extrinsic.clone(),
            );
            let progress = transaction.submit_and_watch().await?;
            progress.wait_for_finalized().await?;
            self.find_signed_extrinsic(signed, self.historical_proxy_address, &original_call)
                .await?;
        }
        if signed.message_id.is_none() {
            if let Some(error) = &signed.finalized_dispatch_error {
                anyhow::ensure!(
                    settlement_pin.is_none(),
                    "HOLD: saved completion original proof failed before message dispatch"
                );
                return Ok((
                    MessageStatus::RetryableNoop {
                        receipt_key,
                        diagnostic: format!(
                            "Finalized signed transaction failed before queuing a message: {error}"
                        ),
                        receipt_observation: Some(FinalizedReceiptObservation {
                            finalized_block_number: signed.inclusion_block_number,
                            finalized_block_hash: signed
                                .inclusion_block_hash
                                .clone()
                                .expect("Finalized dispatch failure has an inclusion hash"),
                        }),
                    },
                    Some(Err(error.clone())),
                ));
            }
        }
        let Some(message_id) = signed
            .message_id
            .as_deref()
            .map(H256::from_str)
            .transpose()?
        else {
            return Ok((
                MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: format!(
                        "Signed transaction {} has no finalized HistoricalProxy message yet",
                        signed.extrinsic_hash
                    ),
                    receipt_observation: None,
                },
                None,
            ));
        };
        let Some((proxy_reply, observation)) = self
            .find_finalized_reply(signed, message_id, self.historical_proxy_address)
            .await?
        else {
            return Ok((
                MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: format!(
                        "HistoricalProxy message {message_id} has no finalized reply yet"
                    ),
                    receipt_observation: signed.inclusion_block_number.map(|number| {
                        FinalizedReceiptObservation {
                            finalized_block_number: Some(number),
                            finalized_block_hash: signed
                                .inclusion_block_hash
                                .clone()
                                .expect("Finalized inclusion number has a matching hash"),
                        }
                    }),
                },
                None,
            ));
        };
        let receiver_reply = match decode_complete_reply::<Redirect>(&proxy_reply) {
            Ok(Ok((receipt_rlp, receiver_reply))) => {
                anyhow::ensure!(
                    receipt_rlp == payload.receipt_rlp,
                    "Finalized HistoricalProxy reply contains another original receipt"
                );
                receiver_reply
            }
            Ok(Err(error)) => {
                let diagnostic = format!("Finalized HistoricalProxy reply failed: {error:?}");
                return Ok((
                    MessageStatus::NeedsReconciliation {
                        receipt_key,
                        diagnostic: diagnostic.clone(),
                        receipt_observation: Some(observation),
                    },
                    Some(Err(diagnostic)),
                ));
            }
            Err(error) => {
                let diagnostic =
                    format!("Could not decode finalized HistoricalProxy reply: {error}");
                return Ok((
                    MessageStatus::NeedsReconciliation {
                        receipt_key,
                        diagnostic: diagnostic.clone(),
                        receipt_observation: Some(observation),
                    },
                    Some(Err(diagnostic)),
                ));
            }
        };
        let status = if self.receiver_route != SubmitReceipt::ROUTE {
            anyhow::ensure!(
                signed.native_reconciliation.is_none()
                    && signed.native_reconciliation_attempts.is_empty(),
                "HOLD: custom consumer cannot authorize manager reconciliation"
            );
            anyhow::ensure!(
                settlement_pin.is_none_or(|pin| pin == &observation),
                "HOLD: custom consumer completion differs from its original finalized reply pin"
            );
            custom_consumer_status(
                &self.receiver_route,
                &receiver_reply,
                receipt_key,
                observation,
            )
        } else {
            match decode_complete_reply::<SubmitReceipt>(&receiver_reply) {
                Ok(Ok(())) | Ok(Err(vft_manager_client::Error::NativeSettlementPending)) => {
                    let (status, native_ready) = self
                        .settlement_status(gear_api, payload, settlement_pin)
                        .await?;
                    if let Some(continuation) = signed.native_reconciliation.as_mut() {
                        anyhow::ensure!(native_ready,
                        "Original native payout or sibling deposits no longer authorize reconciliation");
                        let completed = self
                            .reconcile_native_continuation(
                                gear_api,
                                receipt_key,
                                signed.nonce,
                                continuation,
                                settlement_pin.is_some(),
                            )
                            .await?;
                        if completed {
                            self.settlement_status(gear_api, payload, settlement_pin)
                                .await?
                                .0
                        } else {
                            anyhow::ensure!(
                                settlement_pin.is_none(),
                                "HOLD: saved completion has a failed original continuation"
                            );
                            let archived = *signed
                                .native_reconciliation
                                .take()
                                .expect("continuation was present");
                            anyhow::ensure!(
                                archived.has_terminal_reconciliation(),
                                "HOLD: unknown continuation outcome"
                            );
                            signed.native_reconciliation_attempts.push(archived);
                            // Persist terminal bytes/evidence before signing the next read-only attempt.
                            MessageStatus::Prepared
                        }
                    } else if native_ready && !matches!(status, MessageStatus::Success { .. }) {
                        anyhow::ensure!(
                            settlement_pin.is_none(),
                            "HOLD: historical completion cannot create a continuation"
                        );
                        let call = ReconcileReceipt::encode_call(receipt_key.0, receipt_key.1);
                        let hash = format!("{:#x}", alloy_primitives::keccak256(&call));
                        let continuation = self
                            .prepare_call(gear_api, receipt_key, self.receiver_address, call, hash)
                            .await?;
                        anyhow::ensure!(
                            continuation.nonce
                                > signed
                                    .native_reconciliation_attempts
                                    .last()
                                    .map_or(signed.nonce, |attempt| attempt.nonce),
                            "Native reconciliation nonce must follow the original finalized proof"
                        );
                        signed.native_reconciliation = Some(Box::new(continuation));
                        // Existing Prepared handshake persists the separate raw call before broadcast.
                        MessageStatus::Prepared
                    } else {
                        status
                    }
                }
                Ok(Err(vft_manager_client::Error::UnsupportedEthEvent)) => MessageStatus::Failure(
                    "VFT manager rejected an unsupported Ethereum event before processing it"
                        .into(),
                ),
                Ok(Err(error)) => MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: format!(
                        "VFT manager returned an unresolved finalized error: {error:?}"
                    ),
                    receipt_observation: Some(observation),
                },
                Err(error) => MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic: format!("Could not decode finalized VFT manager reply: {error}"),
                    receipt_observation: Some(observation),
                },
            }
        };
        if let MessageStatus::Success {
            receipt_observation: Some(observation),
        } = &status
        {
            signed.validate_settlement_order(observation)?;
            let minimum = signed
                .finalized_reply
                .as_ref()
                .and_then(|reply| reply.observation.finalized_block_number)
                .into_iter()
                .chain(
                    signed
                        .native_reconciliation
                        .as_ref()
                        .and_then(|continuation| continuation.finalized_reply.as_ref())
                        .and_then(|reply| reply.observation.finalized_block_number),
                )
                .max();
            anyhow::ensure!(
                minimum.is_some_and(|minimum| observation
                    .finalized_block_number
                    .is_some_and(|number| number >= minimum)),
                "Settlement pin predates an original finalized request/reply; HOLD"
            );
        }
        let status = if settlement_pin.is_some() && !matches!(status, MessageStatus::Success { .. })
        {
            MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic: format!(
                    "HOLD: saved completion failed original historical qualification: {status:?}"
                ),
                receipt_observation: settlement_pin.cloned(),
            }
        } else {
            status
        };
        Ok((status, Some(Ok(receiver_reply))))
    }

    async fn reconcile_native_continuation(
        &self,
        gear_api: &GearApi,
        receipt_key: (u64, u64),
        original_nonce: u64,
        signed: &mut SignedSubmission,
        read_only: bool,
    ) -> anyhow::Result<bool> {
        let chain = self.api_provider.client();
        let call = ReconcileReceipt::encode_call(receipt_key.0, receipt_key.1);
        anyhow::ensure!(
            signed.native_reconciliation.is_none()
                && signed.native_reconciliation_attempts.is_empty()
                && signed.chain_genesis_hash == format!("{:#x}", chain.api.genesis_hash())
                && signed.manager == format!("{:#x}", self.receiver_address)
                && signed.historical_proxy == format!("{:#x}", self.historical_proxy_address)
                && (read_only || signed.sender == gear_api.account_id().to_string())
                && signed.receipt_key == receipt_key
                && signed.payload_hash == format!("{:#x}", alloy_primitives::keccak256(&call))
                && signed.nonce > original_nonce,
            "Native reconciliation changed its original non-economic call identity"
        );
        anyhow::ensure!(
            signed.raw_hash_matches(),
            "Native reconciliation raw transaction does not match its original hash"
        );
        if !self
            .find_signed_extrinsic(signed, self.receiver_address, &call)
            .await?
        {
            let account = chain
                .api
                .info_at(
                    &signed.sender,
                    Some(H256::from_str(&signed.scanned_finalized_hash)?),
                )
                .await?;
            if u64::from(account.nonce) > signed.nonce {
                signed.finalized_nonce_consumed = Some(u64::from(account.nonce));
                return Ok(false);
            }
            anyhow::ensure!(
                !read_only,
                "HOLD: historical continuation is absent; no redispatch permitted"
            );
            let transaction = SubmittableTransaction::from_bytes(
                chain.api.deref().clone(),
                signed.raw_extrinsic.clone(),
            );
            transaction
                .submit_and_watch()
                .await?
                .wait_for_finalized()
                .await?;
            self.find_signed_extrinsic(signed, self.receiver_address, &call)
                .await?;
        }
        if signed.message_id.is_none() && signed.finalized_dispatch_error.is_some() {
            return Ok(false);
        }
        let id = signed
            .message_id
            .as_deref()
            .map(H256::from_str)
            .transpose()?
            .ok_or_else(|| {
                anyhow::anyhow!("Original reconciliation has no finalized manager request yet")
            })?;
        let (reply, _) = self
            .find_finalized_reply(signed, id, self.receiver_address)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Original reconciliation has no finalized manager reply yet")
            })?;
        Ok(signed.finalized_reply_error.is_none()
            && matches!(
                decode_complete_reply::<ReconcileReceipt>(&reply),
                Ok(Ok(vft_manager_client::ReceiptStatus::Processed))
            ))
    }

    pub async fn settlement_status(
        &self,
        gear_api: &GearApi,
        payload: &EthToVaraEvent,
        query_pin: Option<&FinalizedReceiptObservation>,
    ) -> anyhow::Result<(MessageStatus, bool)> {
        use vft_manager_client::{ReceiptDepositOutcome, ReceiptStatus};
        use vft_vara_client::traits::NativeEscrow as _;
        anyhow::ensure!(
            self.receiver_route == SubmitReceipt::ROUTE,
            "Unsupported consumer identity/IDL: token completion requires SubmitReceipt"
        );
        let chain = self.api_provider.client();
        let latest = chain.latest_finalized_block().await?;
        let finalized_number = chain.block_hash_to_number(latest).await?;
        let (hash, observation) = if let Some(pin) = query_pin {
            let number = pin
                .finalized_block_number
                .ok_or_else(|| anyhow::anyhow!("HOLD: historical settlement has no height"))?;
            let hash = H256::from_str(&pin.finalized_block_hash)?;
            validate_settlement_pin(
                pin,
                finalized_number,
                chain.block_number_to_hash(number).await?,
            )?;
            (hash, pin.clone())
        } else {
            (
                latest,
                FinalizedReceiptObservation {
                    finalized_block_number: Some(finalized_number),
                    finalized_block_hash: format!("{latest:#x}"),
                },
            )
        };
        let key = (payload.proof_block.block.slot, payload.transaction_index);
        let gas = gear_api.block_gas_limit()?;
        let manager = VftManager::new(GClientRemoting::new(gear_api.clone()));
        let proxy = manager
            .historical_proxy_address()
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        anyhow::ensure!(
            proxy == sails_rs::ActorId::from(self.historical_proxy_address.0),
            "Consumer HistoricalProxy identity changed"
        );
        let ethereum_manager = manager
            .erc_20_manager_address()
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Consumer has no original Ethereum manager identity"))?;
        anyhow::ensure!(
            self.ethereum_manager_address
                .is_none_or(|expected| expected.0 == ethereum_manager.0),
            "Consumer Ethereum manager differs from the immutable source deployment"
        );
        let _config = manager
            .get_config()
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        let native_wrapper = manager
            .native_wrapper()
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        let deposits = manager
            .receipt_deposits(key.0, key.1)
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        validate_deposit_evidence(
            &payload.receipt_rlp,
            key,
            H160::from(ethereum_manager.0),
            self.receiver_address,
            self.historical_proxy_address,
            &deposits,
        )?;
        let mappings = manager
            .vara_to_eth_addresses()
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        let status = manager
            .receipt_status(key.0, key.1)
            .with_gas_limit(gas)
            .at_block(hash.0.into())
            .recv(self.receiver_address.0.into())
            .await?;
        let mut rows_settled = matches!(status, ReceiptStatus::Processed);
        let mut effects_resolved = true;
        let mut has_native = false;
        for row in &deposits {
            anyhow::ensure!(
                row.native == (native_wrapper == Some(row.token_id)),
                "Original native wrapper policy differs from receipt economic evidence"
            );
            anyhow::ensure!(
                mappings
                    .iter()
                    .any(|(token, eth, supply)| *token == row.token_id
                        && *eth == row.eth_token_id
                        && supply.encode() == row.supply.encode()),
                "Original receipt token mapping is unavailable or changed"
            );
            rows_settled &= matches!(
                row.outcome,
                ReceiptDepositOutcome::Settled | ReceiptDepositOutcome::Refunded
            );
            if row.native
                && matches!(
                    row.outcome,
                    ReceiptDepositOutcome::Settled
                        | ReceiptDepositOutcome::Refunded
                        | ReceiptDepositOutcome::NativeQueued
                )
            {
                has_native = true;
                let payout =
                    vft_vara_client::NativeEscrow::new(GClientRemoting::new(gear_api.clone()))
                        .redemption(row.operation_id)
                        .with_gas_limit(gas)
                        .at_block(hash.0.into())
                        .recv(row.token_id)
                        .await?;
                effects_resolved &= payout.as_ref().is_some_and(|payout| {
                    native_payout_resolved(row, self.receiver_address, payout)
                });
            } else {
                effects_resolved &= matches!(row.outcome, ReceiptDepositOutcome::Settled);
            }
        }
        validate_settlement_pin(
            &observation,
            finalized_number,
            chain
                .block_number_to_hash(
                    observation
                        .finalized_block_number
                        .expect("settlement pin has height"),
                )
                .await?,
        )?;
        let native_ready = has_native
            && effects_resolved
            && matches!(status, ReceiptStatus::Reserved | ReceiptStatus::Processed);
        Ok((
            if rows_settled && effects_resolved {
                MessageStatus::Success {
                    receipt_observation: Some(observation),
                }
            } else {
                MessageStatus::NeedsReconciliation {
                receipt_key: key,
                diagnostic: "Original receipt has pending or unknown per-log economic outcomes; original signed request retained".into(),
                receipt_observation: Some(observation),
            }
            },
            native_ready,
        ))
    }

    pub(super) async fn authenticate_archived_continuation(
        &self,
        archived: &SignedSubmission,
    ) -> anyhow::Result<()> {
        let chain = self.api_provider.client();
        let call = ReconcileReceipt::encode_call(archived.receipt_key.0, archived.receipt_key.1);
        let mut observed = archived.clone();
        // An absence archive is qualified by the entire original interval, not its saved cursor alone.
        if archived.inclusion_block_number.is_none() {
            observed.scanned_finalized_number = archived.prepared_finalized_number;
            observed.scanned_finalized_hash = archived.prepared_finalized_hash.clone();
        }
        let included = self
            .find_signed_extrinsic(&mut observed, self.receiver_address, &call)
            .await?;
        if let Some(nonce) = archived.finalized_nonce_consumed {
            let hash = H256::from_str(&archived.scanned_finalized_hash)?;
            let latest = chain
                .block_hash_to_number(chain.latest_finalized_block().await?)
                .await?;
            validate_settlement_pin(
                &FinalizedReceiptObservation {
                    finalized_block_number: Some(archived.scanned_finalized_number),
                    finalized_block_hash: archived.scanned_finalized_hash.clone(),
                },
                latest,
                chain
                    .block_number_to_hash(archived.scanned_finalized_number)
                    .await?,
            )?;
            anyhow::ensure!(
                !included
                    && u64::from(chain.api.info_at(&archived.sender, Some(hash)).await?.nonce)
                        == nonce
                    && nonce > archived.nonce,
                "HOLD: archived finalized absence/consumed nonce changed"
            );
        } else {
            anyhow::ensure!(
                included
                    && observed.inclusion_block_number == archived.inclusion_block_number
                    && observed.inclusion_block_hash == archived.inclusion_block_hash
                    && observed.message_id == archived.message_id
                    && observed.finalized_dispatch_error == archived.finalized_dispatch_error,
                "HOLD: archived original dispatch changed"
            );
            if let Some(id) = archived.message_id.as_deref() {
                self.find_finalized_reply(
                    &mut observed,
                    H256::from_str(id)?,
                    self.receiver_address,
                )
                .await?
                .ok_or_else(|| anyhow::anyhow!("HOLD: archived original reply unavailable"))?;
                anyhow::ensure!(
                    observed.finalized_reply == archived.finalized_reply
                        && observed.finalized_reply_error == archived.finalized_reply_error,
                    "HOLD: archived original terminal reply changed"
                );
            }
        }
        Ok(())
    }

    pub async fn find_signed_extrinsic(
        &self,
        signed: &mut SignedSubmission,
        expected_destination: H256,
        expected_payload: &[u8],
    ) -> anyhow::Result<bool> {
        let chain = self.api_provider.client();
        anyhow::ensure!(
            signed.raw_hash_matches(),
            "HOLD: original raw extrinsic hash changed"
        );
        let decoded = subxt::ext::subxt_core::blocks::Extrinsics::<gsdk::GearConfig>::decode_from(
            vec![signed.raw_extrinsic.clone()],
            chain.api.metadata(),
        )?;
        let extrinsic = decoded
            .iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("HOLD: missing signed envelope"))?;
        let sender = signed.sender.parse::<subxt::utils::AccountId32>()?;
        let mut address = [0; 33];
        address[1..].copy_from_slice(&sender.0);
        let call = extrinsic
            .as_extrinsic::<gear::gear::calls::types::SendMessage>()?
            .ok_or_else(|| anyhow::anyhow!("HOLD: raw original is not Gear.send_message"))?;
        anyhow::ensure!(
            extrinsic.address_bytes() == Some(address.as_slice())
                && extrinsic
                    .transaction_extensions()
                    .and_then(|extensions| extensions.nonce())
                    == Some(signed.nonce)
                && call.destination.as_ref() == expected_destination.as_bytes()
                && call.payload == expected_payload
                && call.value == 0,
            "HOLD: raw signed sender/nonce/destination/payload differs from original identity"
        );
        let prepared_hash = H256::from_str(&signed.prepared_finalized_hash)?;
        anyhow::ensure!(
            chain
                .block_number_to_hash(signed.prepared_finalized_number)
                .await?
                == prepared_hash,
            "Prepared finalized Gear checkpoint changed"
        );
        let cursor_hash = H256::from_str(&signed.scanned_finalized_hash)?;
        anyhow::ensure!(
            chain
                .block_number_to_hash(signed.scanned_finalized_number)
                .await?
                == cursor_hash,
            "Signed extrinsic finalized scan checkpoint changed"
        );
        let finalized = chain
            .block_hash_to_number(chain.latest_finalized_block().await?)
            .await?;
        let sender = signed.sender.parse::<subxt::utils::AccountId32>()?;
        let (start, end) = if let Some(number) = signed.inclusion_block_number {
            anyhow::ensure!(
                number > signed.prepared_finalized_number && number <= finalized,
                "Original signed inclusion is not in the canonical finalized scan interval"
            );
            anyhow::ensure!(
                signed.inclusion_block_hash.as_deref()
                    == Some(format!("{:#x}", chain.block_number_to_hash(number).await?).as_str()),
                "Original signed inclusion hash changed"
            );
            (number, number)
        } else {
            let Some(start) = signed.scanned_finalized_number.checked_add(1) else {
                return Ok(false);
            };
            (start, finalized)
        };
        for number in start..=end {
            let block_hash = chain.block_number_to_hash(number).await?;
            let block = chain.api.blocks().at(block_hash).await?;
            for extrinsic in block.extrinsics().await?.iter() {
                if format!("{:#x}", extrinsic.hash()) != signed.extrinsic_hash {
                    continue;
                }
                let call = extrinsic
                    .as_extrinsic::<gear::gear::calls::types::SendMessage>()?
                    .ok_or_else(|| {
                        anyhow::anyhow!("Original signed extrinsic is not Gear.send_message")
                    })?;
                anyhow::ensure!(call.destination.as_ref() == expected_destination.as_bytes()
                    && call.payload == expected_payload && call.value == 0,
                    "Original finalized extrinsic dispatch differs from the original receipt/proxy/consumer");
                let mut message_id = None;
                let mut dispatch_error = None;
                let mut dispatch_success = false;
                for event in extrinsic.events().await?.iter() {
                    let event = event?;
                    if let Some(queued) = event.as_event::<gear::gear::events::MessageQueued>()? {
                        anyhow::ensure!(queued.source == sender
                            && queued.destination.as_ref() == expected_destination.as_bytes()
                            && matches!(queued.entry, MessageEntry::Handle)
                            && message_id.is_none(),
                            "Original queued message source/destination is not the original signer/proxy");
                        message_id = Some(queued.id.to_string());
                    }
                    if let Some(error) = gear_rpc_client::extrinsic_failure(&event)? {
                        anyhow::ensure!(
                            dispatch_error.is_none(),
                            "Duplicate original dispatch failure"
                        );
                        dispatch_error = Some(error);
                    }
                    if event.pallet_name() == "System" && event.variant_name() == "ExtrinsicSuccess"
                    {
                        anyhow::ensure!(!dispatch_success, "Duplicate original dispatch success");
                        dispatch_success = true;
                    }
                }
                anyhow::ensure!(
                    dispatch_success != dispatch_error.is_some()
                        && dispatch_success == message_id.is_some(),
                    "Original finalized dispatch has missing or conflicting success/failure/queued evidence"
                );
                anyhow::ensure!(
                    signed
                        .message_id
                        .as_ref()
                        .is_none_or(|id| Some(id) == message_id.as_ref()),
                    "Original signed message identity changed"
                );
                anyhow::ensure!(
                    signed
                        .finalized_dispatch_error
                        .as_ref()
                        .is_none_or(|previous| Some(previous) == dispatch_error.as_ref()),
                    "Original finalized dispatch failure changed"
                );
                signed.inclusion_block_number = Some(number);
                signed.inclusion_block_hash = Some(format!("{block_hash:#x}"));
                signed.message_id = message_id;
                signed.finalized_dispatch_error = dispatch_error;
                signed.scanned_finalized_number = number;
                signed.scanned_finalized_hash = format!("{block_hash:#x}");
                return Ok(true);
            }
            signed.scanned_finalized_number = number;
            signed.scanned_finalized_hash = format!("{block_hash:#x}");
        }
        anyhow::ensure!(
            signed.inclusion_block_number.is_none(),
            "Original signed extrinsic is absent from its recorded finalized inclusion"
        );
        Ok(false)
    }

    pub async fn find_finalized_reply(
        &self,
        signed: &mut SignedSubmission,
        message_id: H256,
        expected_source: H256,
    ) -> anyhow::Result<Option<(Vec<u8>, FinalizedReceiptObservation)>> {
        let inclusion_number = signed
            .inclusion_block_number
            .ok_or_else(|| anyhow::anyhow!("Gear message has no finalized inclusion block"))?;
        let chain = self.api_provider.client();
        let cursor_hash = H256::from_str(&signed.reply_scanned_finalized_hash)?;
        anyhow::ensure!(
            chain
                .block_number_to_hash(signed.reply_scanned_finalized_number)
                .await?
                == cursor_hash,
            "Finalized Gear reply scan checkpoint changed"
        );
        let finalized = chain
            .block_hash_to_number(chain.latest_finalized_block().await?)
            .await?;
        // Older journals could advance past an original inner-error reply. Scan
        // from its immutable inclusion without rewinding the discovery cursor.
        let (start, end) = if let Some(reply) = &signed.finalized_reply {
            let number = reply
                .observation
                .finalized_block_number
                .ok_or_else(|| anyhow::anyhow!("Original reply has no finalized height"))?;
            anyhow::ensure!(
                number >= inclusion_number
                    && number <= finalized
                    && reply.observation.finalized_block_hash
                        == format!("{:#x}", chain.block_number_to_hash(number).await?),
                "Original reply is not in its canonical finalized block"
            );
            (number, number)
        } else {
            (inclusion_number, finalized)
        };
        for number in start..=end {
            let block_hash = chain.block_number_to_hash(number).await?;
            for event in chain.metadata_events_at(block_hash).await?.iter() {
                let Some(gear::gear::events::UserMessageSent { message, .. }) =
                    event?.as_event::<gear::gear::events::UserMessageSent>()?
                else {
                    continue;
                };
                let Some(details) = message.details() else {
                    continue;
                };
                if H256::from_slice(details.to_message_id().as_ref()) == message_id {
                    if number >= signed.reply_scanned_finalized_number {
                        signed.reply_scanned_finalized_number = number;
                        signed.reply_scanned_finalized_hash = format!("{block_hash:#x}");
                    }
                    let sender = signed.sender.parse::<subxt::utils::AccountId32>()?;
                    anyhow::ensure!(message.source().as_ref() == expected_source.as_bytes()
                        && message.destination().as_ref() == sender.0,
                        "Original reply source/destination differs from its authenticated actor and signer");
                    let reply_error = (!details.to_reply_code().is_success())
                        .then(|| format!("{:?}", details.to_reply_code()));
                    let evidence = FinalizedReplyEvidence {
                        payload: message.payload_bytes().to_vec(),
                        observation: FinalizedReceiptObservation {
                            finalized_block_number: Some(number),
                            finalized_block_hash: format!("{block_hash:#x}"),
                        },
                    };
                    anyhow::ensure!(
                        signed
                            .finalized_reply
                            .as_ref()
                            .is_none_or(|previous| previous == &evidence),
                        "Original finalized reply bytes or identity changed"
                    );
                    anyhow::ensure!(
                        signed
                            .finalized_reply_error
                            .as_ref()
                            .is_none_or(|previous| Some(previous) == reply_error.as_ref()),
                        "HOLD: original reply code changed"
                    );
                    signed.finalized_reply = Some(evidence.clone());
                    signed.finalized_reply_error = reply_error;
                    if expected_source != self.receiver_address {
                        anyhow::ensure!(
                            signed.finalized_reply_error.is_none(),
                            "HOLD: original proof reply has a failure code"
                        );
                    }
                    return Ok(Some((evidence.payload, evidence.observation)));
                }
            }
            if number >= signed.reply_scanned_finalized_number {
                signed.reply_scanned_finalized_number = number;
                signed.reply_scanned_finalized_hash = format!("{block_hash:#x}");
            }
        }
        anyhow::ensure!(
            signed.finalized_reply.is_none(),
            "Original finalized reply is absent from its recorded block"
        );
        Ok(None)
    }
    fn submission_evidence(
        &self,
        gear_api: &GearApi,
        error: Option<String>,
        manager_reply: Option<Vec<u8>>,
    ) -> SubmissionEvidence {
        SubmissionEvidence {
            observed_at_ms: now_ms(),
            manager: self.receiver_address,
            historical_proxy: self.historical_proxy_address,
            sender: gear_api.account_id().to_string(),
            manager_reply,
            error,
        }
    }

    async fn query_receipt_status(
        &self,
        gear_api: &GearApi,
        receipt_key: (u64, u64),
    ) -> Result<
        (
            vft_manager_client::ReceiptStatus,
            FinalizedReceiptObservation,
        ),
        ReceiptQueryFailure,
    > {
        let api = self.api_provider.client();
        let finalized_hash =
            api.latest_finalized_block()
                .await
                .map_err(|error| ReceiptQueryFailure {
                    diagnostic: format!(
                        "Could not pin receipt query to a finalized Gear block: {error:?}"
                    ),
                    receipt_observation: None,
                })?;
        let finalized_hash_label = format!("{finalized_hash:?}");
        let mut receipt_observation = FinalizedReceiptObservation {
            finalized_block_number: None,
            finalized_block_hash: finalized_hash_label.clone(),
        };
        let finalized_block_number = api
            .block_hash_to_number(finalized_hash)
            .await
            .map_err(|error| ReceiptQueryFailure {
                diagnostic: format!(
                    "Could not resolve the height of finalized Gear block {finalized_hash_label}: {error:?}"
                ),
                receipt_observation: Some(receipt_observation.clone()),
            })?;
        receipt_observation.finalized_block_number = Some(finalized_block_number);
        let status =
            VftManager::new(GClientRemoting::new(gear_api.clone()))
                .receipt_status(receipt_key.0, receipt_key.1)
                .with_gas_limit(gear_api.block_gas_limit().map_err(|error| {
                    ReceiptQueryFailure {
                        diagnostic: format!("Could not get block gas limit: {error:?}"),
                        receipt_observation: Some(receipt_observation.clone()),
                    }
                })?)
                .at_block(finalized_hash.0.into())
                .recv(self.receiver_address.0.into())
                .await
                .map_err(|error| ReceiptQueryFailure {
                    diagnostic: format!("Could not read receipt status: {error:?}"),
                    receipt_observation: Some(receipt_observation.clone()),
                })?;
        Ok((status, receipt_observation))
    }

    async fn update_balance_metric(&self, gear_api: &GearApi) -> anyhow::Result<()> {
        let balance = gear_api
            .total_balance(gear_api.account_id())
            .await
            .map_err(|e| anyhow::anyhow!("Unable to get total balance: {e:?}"))?;

        let balance = balance / UNITS;
        let balance: i64 = balance.try_into().unwrap_or(i64::MAX);

        self.metrics.fee_payer_balance.set(balance);

        Ok(())
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

        let Err(err) = this.run_inner(&mut requests, &mut responses).await else {
            log::warn!("Transaction manager connection terminated, exiting...");
            break;
        };

        log::error!("Gear message sender got an error: {err:?}");
        match this.api_provider.reconnect().await {
            Ok(_) => {
                log::info!("Reconnected to Gear API");
            }

            Err(err) => {
                log::error!("Failed to reconnect to Gear API: {err:?}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::error::TryRecvError;
    use vft_manager_client::ReceiptStatus;

    #[tokio::test]
    #[ignore = "requires retained native token-stack originals and read-only loopback RPC"]
    async fn native_archived_original_dispatch_and_reply() -> anyhow::Result<()> {
        use gear_common::api_provider::ApiProvider;
        let rpc = std::env::var("GEAR_ORIGINAL_RPC")?;
        anyhow::ensure!(
            url::Url::parse(&rpc)?.host_str() == Some("127.0.0.1"),
            "Loopback RPC required"
        );
        let provider = ApiProvider::new(rpc, 0).await?;
        let connection = provider.connection();
        let chain = connection.client();
        anyhow::ensure!(
            format!("{:#x}", chain.api.genesis_hash())
                == "0xa12f631344f61644239b70f9ab53e4842e540d8b0c1e7adb9570c06a62c3af3f",
            "Native source genesis differs"
        );
        let original_hash =
            H256::from_str("0x92ebd056cf9f953ba87d066bc617b33c83d4922dead25f9f0c0068a7b1ddd855")?;
        assert_eq!(chain.block_number_to_hash(813).await?, original_hash);
        assert!(
            chain
                .block_hash_to_number(chain.latest_finalized_block().await?)
                .await?
                >= 813
        );
        let stack: serde_json::Value =
            serde_json::from_slice(&std::fs::read(std::env::var("GEAR_ORIGINAL_STACK")?)?)?;
        let actions = stack["configuration"]["actions"].as_object().unwrap();
        let constructor = &actions["constructor.historicalProxy"]["messages"][0];
        let block = chain.api.blocks().at(original_hash).await?;
        let originals = block.extrinsics().await?;
        let original = originals
            .iter()
            .find(|tx| {
                format!("{:#x}", tx.hash())
                    == "0x6c2f34f324ae9d980cf11fd26e9e31b39f0012145a5e4ccf33c3c1edc2728d22"
            })
            .unwrap();
        assert_eq!(
            original.bytes(),
            hex::decode(
                constructor["submission"]["rawExtrinsic"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x")
            )?
        );
        let mut queued = None;
        for event in original.events().await?.iter() {
            let event = event?;
            assert!(gear_rpc_client::extrinsic_failure(&event)?.is_none());
            if let Some(message) = event.as_event::<gear::gear::events::MessageQueued>()? {
                assert!(matches!(message.entry, MessageEntry::Init));
                assert_eq!(
                    format!("{:#x}", H256::from(message.destination.into_bytes())),
                    constructor["target"].as_str().unwrap()
                );
                queued = Some(message.id);
            }
        }
        let constructor_id = queued.unwrap();
        let mut constructor_reply = false;
        let mut constructor_dispatched = false;
        for event in chain.metadata_events_at(original_hash).await?.iter() {
            let event = event?;
            if let Some(dispatched) = event.as_event::<gear::gear::events::MessagesDispatched>()? {
                for (id, status) in dispatched.statuses {
                    if id == constructor_id {
                        assert!(matches!(
                            status,
                            gear::runtime_types::gear_common::event::DispatchStatus::Success
                        ));
                        constructor_dispatched = true;
                    }
                }
            }
            if let Some(sent) = event.as_event::<gear::gear::events::UserMessageSent>()? {
                if let Some(details) = sent.message.details() {
                    if details.to_message_id() == constructor_id {
                        assert_eq!(
                            format!("{:#x}", H256::from(sent.message.source().into_bytes())),
                            constructor["target"].as_str().unwrap()
                        );
                        assert!(details.to_reply_code().is_success());
                        assert_eq!(
                            sent.message.payload_bytes(),
                            hex::decode(
                                constructor["replyEvidence"]["payload"]
                                    .as_str()
                                    .unwrap()
                                    .trim_start_matches("0x")
                            )?
                        );
                        constructor_reply = true;
                    }
                }
            }
        }
        assert!(constructor_dispatched && constructor_reply);
        let record = actions
            .values()
            .flat_map(|action| action["messages"].as_array().into_iter().flatten())
            .find(|message| message["entry"] == "Handle" && message["replyEvidence"].is_object())
            .unwrap();
        let submission = &record["submission"];
        let target = H256::from_str(record["target"].as_str().unwrap())?;
        let raw = hex::decode(
            submission["rawExtrinsic"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
        )?;
        let sender_bytes = hex::decode(
            submission["sender"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
        )?;
        let sender_account =
            subxt::utils::AccountId32::from(<[u8; 32]>::try_from(sender_bytes).unwrap());
        let prepared_number = submission["scanFromNumber"].as_u64().unwrap() as u32;
        let prepared_hash = submission["scanFromHash"].as_str().unwrap().to_owned();
        let mut signed = SignedSubmission {
            chain_genesis_hash: submission["sourceGenesis"].as_str().unwrap().into(),
            manager: format!("{target:#x}"),
            historical_proxy: format!("{target:#x}"),
            sender: sender_account.to_string(),
            receipt_key: (0, 0),
            payload_hash: String::new(),
            nonce: submission["nonce"].as_u64().unwrap(),
            extrinsic_hash: submission["extrinsicHash"].as_str().unwrap().into(),
            raw_extrinsic: raw,
            prepared_finalized_number: prepared_number,
            prepared_finalized_hash: prepared_hash.clone(),
            scanned_finalized_number: prepared_number,
            scanned_finalized_hash: prepared_hash.clone(),
            reply_scanned_finalized_number: prepared_number,
            reply_scanned_finalized_hash: prepared_hash,
            inclusion_block_number: Some(
                record["inclusionEvidence"]["blockNumber"].as_u64().unwrap() as u32,
            ),
            inclusion_block_hash: Some(
                record["inclusionEvidence"]["blockHash"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
            message_id: None,
            finalized_dispatch_error: None,
            finalized_reply: None,
            native_reconciliation: None,
            native_reconciliation_attempts: vec![],
            finalized_nonce_consumed: None,
            finalized_reply_error: None,
        };
        let reader = MessageSender::new(target, vec![], target, connection, "//Alice".into(), None);
        let payload = hex::decode(record["payload"].as_str().unwrap().trim_start_matches("0x"))?;
        assert!(
            reader
                .find_signed_extrinsic(&mut signed, target, &payload)
                .await?
        );
        let message_id = H256::from_str(signed.message_id.as_deref().unwrap())?;
        let reply = reader
            .find_finalized_reply(&mut signed, message_id, target)
            .await?
            .unwrap();
        assert_eq!(
            reply.0,
            hex::decode(
                record["replyEvidence"]["payload"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x")
            )?
        );
        let saved = signed.clone();
        assert!(
            reader
                .find_signed_extrinsic(&mut signed, target, &payload)
                .await?
        );
        assert_eq!(
            reader
                .find_finalized_reply(&mut signed, message_id, target)
                .await?
                .unwrap(),
            reply
        );
        assert_eq!(signed, saved);
        assert!(reader
            .find_signed_extrinsic(&mut signed.clone(), target, &[0xff])
            .await
            .is_err());
        assert!(reader
            .find_finalized_reply(&mut signed.clone(), message_id, H256::zero())
            .await
            .is_err());
        let mut changed = signed.clone();
        changed.raw_extrinsic.push(0);
        assert!(reader
            .find_signed_extrinsic(&mut changed, target, &payload)
            .await
            .is_err());
        let mut changed = signed.clone();
        changed.inclusion_block_hash = Some(format!("{:#x}", H256::zero()));
        assert!(reader
            .find_signed_extrinsic(&mut changed, target, &payload)
            .await
            .is_err());
        let mut changed = signed.clone();
        changed
            .finalized_reply
            .as_mut()
            .unwrap()
            .observation
            .finalized_block_hash = format!("{:#x}", H256::zero());
        assert!(reader
            .find_finalized_reply(&mut changed, message_id, target)
            .await
            .is_err());
        println!("Native original block813 constructor and Handle {} inclusion {:?} reply {:?} requalified without signing",
            signed.extrinsic_hash, signed.inclusion_block_number, reply.1);
        Ok(())
    }

    #[test]
    fn historical_pin_requires_canonical_finalized_hash_and_original_order() {
        let hash = H256::repeat_byte(20);
        let pin = FinalizedReceiptObservation {
            finalized_block_number: Some(20),
            finalized_block_hash: format!("{hash:#x}"),
        };
        assert!(validate_settlement_pin(&pin, 74, hash).is_ok());
        assert!(validate_settlement_pin(&pin, 19, hash).is_err());
        assert!(validate_settlement_pin(&pin, 74, H256::repeat_byte(21)).is_err());
        let missing = FinalizedReceiptObservation {
            finalized_block_number: None,
            finalized_block_hash: pin.finalized_block_hash,
        };
        assert!(validate_settlement_pin(&missing, 74, hash).is_err());
    }

    #[test]
    fn reply_frames_require_exact_routes_and_complete_consumption() {
        let inner: <SubmitReceipt as ActionIo>::Reply = Ok(());
        let mut inner_bytes = SubmitReceipt::ROUTE.to_vec();
        inner_bytes.extend(inner.encode());
        assert!(matches!(
            decode_complete_reply::<SubmitReceipt>(&inner_bytes),
            Ok(Ok(()))
        ));
        let outer: <Redirect as ActionIo>::Reply = Ok((vec![1, 2, 3], inner_bytes.clone()));
        let mut outer_bytes = Redirect::ROUTE.to_vec();
        outer_bytes.extend(outer.encode());
        assert!(
            matches!(decode_complete_reply::<Redirect>(&outer_bytes), Ok(Ok((receipt, _))) if receipt == vec![1, 2, 3])
        );
        outer_bytes.push(0);
        inner_bytes.push(0);
        assert!(decode_complete_reply::<Redirect>(&outer_bytes).is_err());
        assert!(decode_complete_reply::<SubmitReceipt>(&inner_bytes).is_err());
        assert!(decode_complete_reply::<SubmitReceipt>(&outer_bytes).is_err());
        let error: <SubmitReceipt as ActionIo>::Reply = Err(vft_manager_client::Error::Paused);
        let mut bytes = SubmitReceipt::ROUTE.to_vec();
        bytes.extend(error.encode());
        assert!(matches!(
            decode_complete_reply::<SubmitReceipt>(&bytes),
            Ok(Err(vft_manager_client::Error::Paused))
        ));
    }

    #[test]
    fn custom_consumer_requires_exact_route_and_successful_result_not_manager_settlement() {
        let route = ("Ping", "SubmitReceipt").encode();
        let observation = FinalizedReceiptObservation {
            finalized_block_number: Some(20),
            finalized_block_hash: format!("{:#x}", H256::repeat_byte(20)),
        };
        // Ping returns a Delivery, not the manager's Result<(), Error>.
        let delivery = (H256::repeat_byte(7), vec![1u8, 2, 3]);
        let mut reply = route.clone();
        reply.extend(Result::<_, u8>::Ok(delivery).encode());
        assert!(matches!(
            custom_consumer_status(&route, &reply, (17, 4), observation.clone()),
            MessageStatus::Success { receipt_observation: Some(pin) } if pin == observation
        ));
        let mut rejected = route.clone();
        rejected.extend(Result::<(), u8>::Err(7).encode());
        let mut invalid_result = route.clone();
        invalid_result.push(2);
        let mut manager_reply = SubmitReceipt::ROUTE.to_vec();
        manager_reply.extend(Result::<(), u8>::Ok(()).encode());
        for (expected, bytes) in [
            (route.as_slice(), rejected.as_slice()),
            (route.as_slice(), invalid_result.as_slice()),
            (route.as_slice(), route.as_slice()),
            (route.as_slice(), manager_reply.as_slice()),
            (SubmitReceipt::ROUTE, manager_reply.as_slice()),
        ] {
            assert!(matches!(
                custom_consumer_status(expected, bytes, (17, 4), observation.clone()),
                MessageStatus::NeedsReconciliation { .. }
            ));
        }
    }

    #[test]
    fn every_original_deposit_and_native_payout_must_match() {
        use alloy::sol_types::SolEvent;
        use alloy_consensus::{Receipt, ReceiptWithBloom};
        use alloy_primitives::{Address, Log};
        use alloy_rlp::Encodable;
        use ethereum_client::abi::IERC20Manager::BridgingRequested;
        use sails_rs::{ActorId, MessageId};
        use vft_manager_client::{ReceiptDepositOutcome, ReceiptDepositState, TokenSupply};
        let ethereum_manager = H160::repeat_byte(1);
        let manager = H256::repeat_byte(2);
        let proxy = H256::repeat_byte(3);
        let key = (17, 4);
        let events: Vec<_> = (0..2)
            .map(|n| BridgingRequested {
                token: Address::from([4; 20]),
                from: Address::from([5; 20]),
                to: FixedBytes::from([6 + n; 32]),
                amount: alloy_primitives::U256::from(123 + n as u64),
            })
            .collect();
        let logs = events
            .iter()
            .map(|event| Log {
                address: Address::from(ethereum_manager.0),
                data: event.encode_log_data(),
            })
            .collect();
        let receipt = ethereum_common::utils::ReceiptEnvelope::Legacy(ReceiptWithBloom::new(
            Receipt {
                status: true.into(),
                cumulative_gas_used: 1,
                logs,
            },
            Default::default(),
        ));
        let mut bytes = Vec::new();
        receipt.encode(&mut bytes);
        let receipt_hash = H256::from(alloy_primitives::keccak256(&bytes).0);
        let rows: Vec<_> = events
            .iter()
            .enumerate()
            .map(|(i, event)| ReceiptDepositState {
                log_index: i as u64,
                sender: event.from.0 .0.into(),
                receiver: event.to.0.into(),
                token_id: ActorId::from([7; 32]),
                eth_token_id: event.token.0 .0.into(),
                amount: sails_rs::U256::from_little_endian(event.amount.as_le_slice()),
                supply: TokenSupply::Gear,
                native: false,
                operation_id: alloy_primitives::keccak256(
                    (
                        b"vara/native-escrow/v1",
                        ActorId::from(manager.0),
                        ActorId::from(proxy.0),
                        ethereum_manager,
                        key.0,
                        key.1,
                        i as u64,
                        receipt_hash,
                    )
                        .encode(),
                )
                .0
                .into(),
                child: Some(MessageId::from([8; 32])),
                outcome: ReceiptDepositOutcome::Settled,
            })
            .collect();
        assert!(
            validate_deposit_evidence(&bytes, key, ethereum_manager, manager, proxy, &rows).is_ok()
        );
        // The retained hash-20 rows qualify; evicted latest rows must never substitute for them.
        assert!(
            validate_deposit_evidence(&bytes, key, ethereum_manager, manager, proxy, &[]).is_err()
        );
        assert!(validate_deposit_evidence(
            &bytes,
            key,
            ethereum_manager,
            manager,
            proxy,
            &rows[..1]
        )
        .is_err());
        let mut wrong = rows.clone();
        wrong.swap(0, 1);
        assert!(
            validate_deposit_evidence(&bytes, key, ethereum_manager, manager, proxy, &wrong)
                .is_err()
        );
        for field in [
            "sender",
            "receiver",
            "amount",
            "operation",
            "log",
            "token",
            "child",
        ] {
            let mut wrong = rows.clone();
            match field {
                "sender" => wrong[0].sender = [0; 20].into(),
                "receiver" => wrong[0].receiver = ActorId::zero(),
                "amount" => wrong[0].amount += 1.into(),
                "operation" => wrong[0].operation_id = [0; 32].into(),
                "log" => wrong[0].log_index += 1,
                "token" => wrong[0].eth_token_id = [0; 20].into(),
                "child" => wrong[0].child = None,
                _ => unreachable!(),
            }
            assert!(
                validate_deposit_evidence(&bytes, key, ethereum_manager, manager, proxy, &wrong)
                    .is_err(),
                "{field}"
            );
        }
        assert!(validate_deposit_evidence(
            &bytes,
            (key.0 + 1, key.1),
            ethereum_manager,
            manager,
            proxy,
            &rows
        )
        .is_err());
        bytes.push(0);
        assert!(
            validate_deposit_evidence(&bytes, key, ethereum_manager, manager, proxy, &rows)
                .is_err()
        );
        let row = &rows[0];
        let mut payout = vft_vara_client::Redemption {
            from: ActorId::from(manager.0),
            to: row.receiver,
            amount: row.amount,
            child: MessageId::from([9; 32]),
            status: vft_vara_client::PayoutStatus::Queued,
            returned_value: 0,
        };
        assert!(!native_payout_resolved(row, manager, &payout));
        payout.status = vft_vara_client::PayoutStatus::Delivered;
        assert!(native_payout_resolved(row, manager, &payout));
        payout.returned_value = 1;
        assert!(!native_payout_resolved(row, manager, &payout));
        payout.returned_value = 0;
        payout.to = ActorId::zero();
        assert!(!native_payout_resolved(row, manager, &payout));
        payout.to = row.receiver;
        payout.status = vft_vara_client::PayoutStatus::Refunded;
        payout.returned_value = row.amount.as_u128();
        assert!(!native_payout_resolved(row, manager, &payout)); // Never relabel Settled as refunded.
        let mut refunded = row.clone();
        refunded.outcome = ReceiptDepositOutcome::Refunded;
        assert!(native_payout_resolved(&refunded, manager, &payout));
        payout.returned_value -= 1;
        assert!(!native_payout_resolved(&refunded, manager, &payout));
        payout.returned_value += 1;
        payout.status = vft_vara_client::PayoutStatus::Returned;
        assert!(!native_payout_resolved(&refunded, manager, &payout));
        payout.status = vft_vara_client::PayoutStatus::Ambiguous;
        assert!(!native_payout_resolved(&refunded, manager, &payout));
    }

    #[tokio::test]
    #[ignore = "requires the original read-only Gear RPC and retained manager/receipt identities"]
    async fn read_only_finalized_receipt_status_never_completes_an_original_request(
    ) -> anyhow::Result<()> {
        use gear_common::api_provider::ApiProvider;
        let provider = ApiProvider::new(std::env::var("GEAR_COMPLETION_RPC")?, 0).await?;
        let manager = H256::from_str(&std::env::var("GEAR_COMPLETION_MANAGER")?)?;
        let key = (
            std::env::var("GEAR_COMPLETION_SLOT")?.parse()?,
            std::env::var("GEAR_COMPLETION_TX_INDEX")?.parse()?,
        );
        let mut sender = MessageSender::new(
            manager,
            SubmitReceipt::ROUTE.to_vec(),
            H256::zero(),
            provider.connection(),
            "//Alice".into(),
            None,
        );
        let client = sender.api_provider.gclient_client("//Alice")?;
        let (status, observation) = sender
            .query_receipt_status(&client, key)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.diagnostic))?;
        println!("Read-only original receipt {key:?}: {status:?} at {observation}");
        assert!(matches!(
            receipt_query_result(key, Ok((status, observation))),
            MessageStatus::NeedsReconciliation { .. }
        ));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires a read-only Gear RPC proxy failing the second System.Account lookup"]
    async fn post_dequeue_balance_fault_reports_original_request() -> anyhow::Result<()> {
        use gear_common::api_provider::ApiProvider;
        use std::time::Duration;

        let endpoint = std::env::var("GEAR_BALANCE_FAULT_RPC")?;
        let provider = ApiProvider::new(endpoint, 0).await?;
        let mut sender = MessageSender::new(
            H256::zero(),
            Vec::new(),
            H256::zero(),
            provider.connection(),
            "//Alice".into(),
            None,
        );
        let (requests_tx, mut requests_rx) = unbounded_channel();
        let (mut responses_tx, mut responses_rx) = unbounded_channel();
        let tx_uuid = Uuid::now_v7();
        // The injected balance error must occur before any proof processing or signing.
        let payload = super::super::tx_manager::tests::receipt_fixtures()
            .remove(0)
            .1;
        requests_tx.send(Request::PrepareMessage {
            tx_uuid,
            tx_hash: FixedBytes::ZERO,
            payload: Box::new(payload),
        })?;
        drop(requests_tx);

        let error = tokio::time::timeout(
            Duration::from_secs(15),
            sender.run_inner(&mut requests_rx, &mut responses_tx),
        )
        .await?
        .expect_err("the injected balance RPC failure must propagate");
        assert!(format!("{error:#}").contains("balance-fault-after-dequeue"));
        assert!(matches!(
            requests_rx.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
        let response = responses_rx
            .try_recv()
            .expect("consumed request lost its failure response");
        assert_eq!(response.tx_uuid, tx_uuid);
        assert!(matches!(
            response.status, MessageStatus::PrepareFailed(diagnostic)
                if diagnostic.contains("balance-fault-after-dequeue")
        ));
        assert!(response.signed_submission.is_none());
        Ok(())
    }

    #[test]
    fn receipt_queries_retain_finalized_height_and_hash() {
        let key = (17, 3);
        let observation = FinalizedReceiptObservation {
            finalized_block_number: Some(73),
            finalized_block_hash: "0x1234".to_owned(),
        };

        for status in [
            ReceiptStatus::Processed,
            ReceiptStatus::Reserved,
            ReceiptStatus::Unknown,
        ] {
            match receipt_query_result(key, Ok((status, observation.clone()))) {
                MessageStatus::NeedsReconciliation {
                    receipt_key,
                    diagnostic,
                    receipt_observation: Some(actual),
                } => {
                    assert_eq!(receipt_key, key);
                    assert_eq!(actual, observation);
                    assert!(diagnostic.contains("finalized Gear block 73 (0x1234)"));
                    assert!(diagnostic.contains("will not be resubmitted"));
                }
                other => panic!("unexpected receipt result: {other:?}"),
            }
        }
    }

    #[test]
    fn pinned_and_unpinned_query_errors_retain_available_evidence() {
        let key = (17, 3);
        let observation = FinalizedReceiptObservation {
            finalized_block_number: Some(73),
            finalized_block_hash: "0x1234".to_owned(),
        };
        let pinned_error = ReceiptQueryFailure {
            diagnostic: "state read failed".into(),
            receipt_observation: Some(observation.clone()),
        };
        match receipt_query_result(key, Err(pinned_error)) {
            MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic,
                receipt_observation: Some(actual),
            } => {
                assert_eq!(receipt_key, key);
                assert_eq!(actual, observation);
                assert!(diagnostic.contains("finalized Gear block 73 (0x1234)"));
                assert!(diagnostic.contains("state read failed"));
                assert!(diagnostic.contains("will not be resubmitted"));
            }
            other => panic!("unexpected receipt result: {other:?}"),
        }
        let pinned_hash_without_height = FinalizedReceiptObservation {
            finalized_block_number: None,
            finalized_block_hash: "0x5678".to_owned(),
        };
        let height_error = ReceiptQueryFailure {
            diagnostic: "height resolution failed".into(),
            receipt_observation: Some(pinned_hash_without_height.clone()),
        };
        match receipt_query_result(key, Err(height_error)) {
            MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic,
                receipt_observation: Some(actual),
            } => {
                assert_eq!(receipt_key, key);
                assert_eq!(actual, pinned_hash_without_height);
                assert!(diagnostic.contains("finalized Gear block 0x5678 (height unavailable)"));
                assert!(diagnostic.contains("height resolution failed"));
            }
            other => panic!("unexpected receipt result: {other:?}"),
        }

        let unpinned_error = ReceiptQueryFailure {
            diagnostic: "finalized head unavailable".into(),
            receipt_observation: None,
        };
        match receipt_query_result(key, Err(unpinned_error)) {
            MessageStatus::NeedsReconciliation {
                receipt_key,
                diagnostic,
                receipt_observation: None,
            } => {
                assert_eq!(receipt_key, key);
                assert!(diagnostic.contains("finalized head unavailable"));
                assert!(diagnostic.contains("will not be resubmitted"));
            }
            other => panic!("unexpected receipt result: {other:?}"),
        }
    }
}
