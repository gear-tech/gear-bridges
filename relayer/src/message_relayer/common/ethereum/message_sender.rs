use crate::{
    common::{is_transport_error_recoverable, BASE_RETRY_DELAY},
    lane_policy::LanePolicy,
    message_relayer::common::RelayedMerkleRoot,
};
use ethereum_client::{
    abi::IMessageQueue::IMessageQueueErrors, transaction_identity, EthApi, PreparedContentMessage,
    SubmissionGuard, TxHash,
};
use gear_rpc_client::dto::{MerkleProof, Message};
use prometheus::{Gauge, IntCounter};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot, watch,
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
    pub lane_policy: Option<LanePolicy>,
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

#[derive(Clone)]
struct Original {
    uuid: Uuid,
    message: Message,
    submission: PreparedContentMessage,
}

type Originals = BTreeMap<(u64, Uuid), Arc<Original>>;

pub struct MessageSenderIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
    original_owners: watch::Sender<HashSet<Uuid>>,
    original_registrations: watch::Sender<Originals>,
}

impl MessageSenderIo {
    pub fn new(requests: UnboundedSender<Request>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
            responses,
            original_owners: watch::channel(HashSet::new()).0,
            original_registrations: watch::channel(BTreeMap::new()).0,
        }
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }

    pub fn retain_original(&self, uuid: Uuid) {
        self.original_owners.send_modify(|owners| {
            owners.insert(uuid);
        });
    }

    /// Restore only the original signed identity; root/proof are recovered from its bytes.
    pub fn register_original(
        &self,
        uuid: Uuid,
        message: Message,
        submission: PreparedContentMessage,
    ) -> bool {
        self.retain_original(uuid);
        if super::status_fetcher::validate_original_submission(&message, &submission).is_err() {
            return false;
        }
        let mut matches = true;
        self.original_registrations.send_modify(|originals| {
            if let Some(original) = originals.values().find(|original| original.uuid == uuid) {
                matches = original.submission == submission && original.message == message;
            } else if originals
                .keys()
                .any(|(nonce, _)| *nonce == submission.nonce)
            {
                matches = false;
            } else {
                originals.insert(
                    (submission.nonce, uuid),
                    Arc::new(Original {
                        uuid,
                        message,
                        submission,
                    }),
                );
            }
        });
        matches
    }

    pub fn reconcile_original(&self, uuid: Uuid) {
        self.original_owners.send_modify(|owners| {
            owners.remove(&uuid);
        });
        self.original_registrations.send_modify(|originals| {
            originals.retain(|_, original| original.uuid != uuid);
        });
    }

    pub fn send(
        &mut self,
        message: Message,
        relayed_root: RelayedMerkleRoot,
        proof: MerkleProof,
        tx_uuid: Uuid,
        prepared: Option<PreparedContentMessage>,
        lane_policy: Option<LanePolicy>,
    ) -> bool {
        self.requests
            .send(Request {
                message,
                relayed_root,
                proof,
                tx_uuid,
                prepared,
                lane_policy,
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
        let (original_owners, ownership) = watch::channel(HashSet::new());
        let (original_registrations, registrations) = watch::channel(BTreeMap::new());
        tokio::task::spawn(task(
            self,
            requests_tx.downgrade(),
            requests_rx,
            responses_tx,
            ownership,
            registrations,
        ));

        MessageSenderIo {
            requests: requests_tx,
            responses: responses_rx,
            original_owners,
            original_registrations,
        }
    }
}

async fn task(
    mut this: MessageSender,
    retry_requests: mpsc::WeakUnboundedSender<Request>,
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
    mut ownership: watch::Receiver<HashSet<Uuid>>,
    mut registrations: watch::Receiver<Originals>,
) {
    match this.eth_api.get_approx_balance().await {
        Ok(balance) => this.metrics.fee_payer_balance.set(balance),
        Err(error) => log::warn!("Failed to update Ethereum fee payer balance metric: {error}"),
    }
    let mut submission_guard = None;
    loop {
        if ownership.borrow().is_empty() {
            submission_guard.take();
        } else {
            if submission_guard.is_none() {
                submission_guard = Some(this.eth_api.reserve_submission().await);
            }
            if !wait_for_original_completion(
                &mut this,
                submission_guard.as_ref().unwrap(),
                &mut ownership,
                &mut registrations,
                None,
                &responses,
            )
            .await
            {
                retain_until_shutdown(submission_guard).await;
                return;
            }
            continue;
        }
        let request = tokio::select! {
            biased;
            changed = ownership.changed() => {
                if changed.is_err() { retain_until_shutdown(submission_guard).await; return; }
                continue;
            },
            request = requests.recv() => match request {
                Some(request) => request,
                None => { retain_until_shutdown(submission_guard).await; return; }
            },
        };
        let mut prepared = request.prepared.clone();
        loop {
            if submission_guard.is_none() {
                submission_guard = Some(this.eth_api.reserve_submission().await);
            }
            let needs_reconciliation = prepared.is_none() && !ownership.borrow().is_empty();
            if needs_reconciliation
                && !wait_for_original_completion(
                    &mut this,
                    submission_guard.as_ref().unwrap(),
                    &mut ownership,
                    &mut registrations,
                    None,
                    &responses,
                )
                .await
            {
                retain_until_shutdown(submission_guard).await;
                return;
            }
            match process_request(
                &mut this,
                &request,
                submission_guard.as_ref().unwrap(),
                &mut prepared,
                &responses,
            )
            .await
            {
                Ok(Outcome::Ready | Outcome::Hold) => break,
                Ok(Outcome::Stop) => {
                    retain_until_shutdown(submission_guard).await;
                    return;
                }
                Err(error) => {
                    let delay = BASE_RETRY_DELAY * 6;
                    log::error!(
                        "Ethereum message sender failed: {error:?}; retaining any saved original"
                    );
                    if prepared.is_some() {
                        // Its durable watcher already owns outcome accounting. Only retry exact bytes.
                        break;
                    }
                    submission_guard.take();
                    if is_retryable_contract_submission(&error) {
                        let request = request.clone();
                        let retry_requests = retry_requests.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(delay).await;
                            if let Some(requests) = retry_requests.upgrade() {
                                let _ = requests.send(request);
                            }
                        });
                        break;
                    }
                    tokio::time::sleep(delay).await;
                    if let Ok(api) = this.eth_api.reconnect().await {
                        this.eth_api = api;
                    }
                }
            }
        }
        if let Some(submission) = prepared {
            let active = Arc::new(Original {
                uuid: request.tx_uuid,
                message: request.message,
                submission,
            });
            if !wait_for_original_completion(
                &mut this,
                submission_guard.as_ref().unwrap(),
                &mut ownership,
                &mut registrations,
                Some(active),
                &responses,
            )
            .await
            {
                retain_until_shutdown(submission_guard).await;
                return;
            }
        }
    }
}

async fn retain_until_shutdown(guard: Option<SubmissionGuard>) {
    if let Some(guard) = guard {
        std::future::pending::<()>().await;
        drop(guard);
    }
}

async fn wait_for_original_completion(
    this: &mut MessageSender,
    guard: &SubmissionGuard,
    ownership: &mut watch::Receiver<HashSet<Uuid>>,
    registrations: &mut watch::Receiver<Originals>,
    active: Option<Arc<Original>>,
    responses: &UnboundedSender<Response>,
) -> bool {
    let mut originals = registrations.borrow_and_update().clone();
    if let Some(original) = &active {
        originals.insert((original.submission.nonce, original.uuid), original.clone());
    }
    let mut retry = tokio::time::interval(BASE_RETRY_DELAY * 6);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        if ownership.borrow().is_empty() {
            return true;
        }
        tokio::select! {
            biased;
            _ = responses.closed() => return false,
            changed = ownership.changed() => {
                if changed.is_err() { return false; }
            },
            changed = registrations.changed() => {
                if changed.is_err() { return false; }
                originals = registrations.borrow_and_update().clone();
                if let Some(original) = &active {
                    originals.insert((original.submission.nonce, original.uuid), original.clone());
                }
                retry.reset_immediately();
            },
            _ = retry.tick() => {
                let mut reconnect = false;
                // Every restored original gets a turn, in nonce order, under this same guard.
                for original in originals.values() {
                    if !ownership.borrow().contains(&original.uuid) { continue; }
                    if let Err(error) = retry_original(&this.eth_api, guard, original).await {
                        log::warn!("HOLD: original outbound {} remains owned: {error:?}", original.submission.hash);
                        reconnect = true;
                    }
                }
                if reconnect {
                    match this.eth_api.reconnect().await {
                        Ok(api) => this.eth_api = api,
                        Err(error) => log::warn!("Original outbound retry reconnect failed: {error}"),
                    }
                }
            },
        }
    }
}

/// Authenticate the saved message/domain without requiring discarded discovery proof state.
fn validate_original(api: &EthApi, original: &Original) -> anyhow::Result<()> {
    let signed = &original.submission;
    let call = super::status_fetcher::original_call(&original.message, signed)?;
    api.validate_prepared_content_message(
        signed,
        call.blockNumber.try_into()?,
        call.totalLeaves.try_into()?,
        call.leafIndex.try_into()?,
        original.message.nonce_be,
        original.message.source,
        original.message.destination,
        original.message.payload.clone(),
        call.proof.into_iter().map(|hash| hash.0).collect(),
    )
}

async fn retry_original(
    api: &EthApi,
    guard: &SubmissionGuard,
    original: &Original,
) -> anyhow::Result<()> {
    validate_original(api, original)?;
    let signed = &original.submission;
    // Receipt finality takes precedence over a consumed nonce or a competitor's processed bit.
    if let Some(included) = api.get_finalized_receipt(signed.hash).await? {
        super::status_fetcher::validate_receipt_authentication(
            signed,
            &included.receipt,
            &included.inclusion,
            &included.checkpoint,
        )?;
        anyhow::ensure!(
            included.receipt.block_number == Some(included.included_block_number)
                && included.receipt.block_hash == Some(included.included_block_hash),
            "finalized original receipt identity differs from saved submission"
        );
        // The manager, not this broadcast owner, saves success/revert evidence and releases ownership.
        return Ok(());
    }
    let snapshot = api
        .get_finalized_submission_snapshot(&signed.raw_transaction)
        .await?
        .ok_or_else(|| anyhow::anyhow!("canonical finalized submission snapshot is pending"))?;
    if let Some(observed) = transaction_identity(api.raw_provider(), signed.hash).await? {
        anyhow::ensure!(
            observed.hash == signed.hash
                && observed.from == signed.sender
                && observed.nonce == signed.nonce
                && observed.to == Some(signed.contract),
            "RPC outbound transaction differs from saved signed identity"
        );
    } else if !snapshot.pinned_nonce_consumed() {
        // Even a processed message must consume its original nonce; a competitor cannot release it.
        let hash = api
            .broadcast_prepared_content_message(guard, signed)
            .await?;
        anyhow::ensure!(
            hash == signed.hash,
            "RPC changed original outbound transaction hash"
        );
    }
    Ok(())
}

enum Outcome {
    Ready,
    Stop,
    Hold,
}

/// A saved signature may only be reconciled or broadcast verbatim after durable handoff.
async fn process_request(
    this: &mut MessageSender,
    request: &Request,
    submission_guard: &SubmissionGuard,
    prepared: &mut Option<PreparedContentMessage>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<Outcome> {
    let message = &request.message;
    let root = request.relayed_root;
    let proof = &request.proof;
    let tx_uuid = request.tx_uuid;
    let total_leaves = u32::try_from(proof.num_leaves)?;
    let leaf_index = u32::try_from(proof.leaf_index)?;
    let fresh = prepared.is_none();
    let Some(policy) = request.lane_policy.as_ref() else {
        return Ok(report_hold(tx_uuid, "HOLD: outbound lane has no independently approved persisted effect policy".into(), responses));
    };
    if let Err(error) = policy.validate_for_api(&this.eth_api, message) {
        return Ok(report_hold(tx_uuid, error.to_string(), responses));
    }
    if fresh && policy.original_hash.is_some() {
        return Ok(report_hold(tx_uuid, "HOLD: historical original approval cannot authorize new signing".into(), responses));
    }
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
                return Ok(
                    if responses
                        .send(Response::Failed(
                            tx_uuid,
                            format!("Failed to prepare content message before broadcast: {error}"),
                        ))
                        .is_ok()
                    {
                        Outcome::Ready
                    } else {
                        Outcome::Stop
                    },
                );
            }
        };
        *prepared = Some(signed);
    }
    let signed = prepared
        .as_ref()
        .expect("submission was prepared or restored");
    if let Err(error) = policy.validate(message, signed) {
        return Ok(report_hold(tx_uuid, error.to_string(), responses));
    }
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
        return Ok(Outcome::Stop);
    }
    // Durable handoff is the only new submission/watch boundary. The ownership loop
    // handles both initial broadcast and every later retry without emitting it again.
    if fresh {
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
            return Ok(Outcome::Stop);
        }
    }
    match this.eth_api.get_approx_balance().await {
        Ok(fee_payer_balance) => this.metrics.fee_payer_balance.set(fee_payer_balance),
        Err(e) => log::warn!("Failed to update Ethereum fee payer balance metric: {e}"),
    }
    Ok(Outcome::Ready)
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

fn report_hold(tx_uuid: Uuid, reason: String, responses: &UnboundedSender<Response>) -> Outcome {
    log::error!("{reason}");
    if responses.send(Response::Hold(tx_uuid, reason)).is_ok() {
        Outcome::Hold
    } else {
        Outcome::Stop
    }
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

    #[tokio::test]
    async fn original_ownership_retains_restart_and_hold_fences() {
        use super::{report_hold, MessageSenderIo, Outcome};
        use futures::{pin_mut, poll};
        use tokio::sync::mpsc;
        use uuid::Uuid;
        let (requests, _received) = mpsc::unbounded_channel();
        let (responses, received) = mpsc::unbounded_channel();
        let io = MessageSenderIo::new(requests, received);
        let original = Uuid::new_v4();
        let restored = Uuid::new_v4();
        io.retain_original(original);
        io.retain_original(restored);
        let mut ownership = io.original_owners.subscribe();
        let unsigned = ownership.wait_for(|owners| owners.is_empty());
        pin_mut!(unsigned);
        assert!(
            poll!(&mut unsigned).is_pending(),
            "RPC pending nonce is not ownership"
        );
        io.reconcile_original(Uuid::new_v4());
        assert!(
            poll!(&mut unsigned).is_pending(),
            "stale completion cannot release an original"
        );
        assert!(matches!(
            report_hold(original, "HOLD: no canonical receipt".into(), &responses),
            Outcome::Hold
        ));
        assert!(
            poll!(&mut unsigned).is_pending(),
            "HOLD retains nonce ownership"
        );
        io.reconcile_original(original);
        assert!(
            poll!(&mut unsigned).is_pending(),
            "all restart originals must reconcile first"
        );
        io.reconcile_original(restored);
        assert!(unsigned.await.is_ok());
        drop(io);
        let mut ownership =
            tokio::sync::watch::channel(std::collections::HashSet::from([original])).1;
        assert!(
            ownership
                .wait_for(|owners| owners.is_empty())
                .await
                .is_err(),
            "shutdown cannot authorize unsigned work"
        );
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

    async fn original(nonce: u64) -> super::Original {
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
        let message = gear_rpc_client::dto::Message {
            nonce_be: U256::from(7).to_be_bytes(),
            source: [1; 32],
            destination: [2; 20],
            payload: vec![3, 4],
        };
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
            chain_id: 31337,
            nonce,
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
            chain_id: 31337,
            contract,
            sender: signer.address(),
            nonce,
            hash: alloy::primitives::keccak256(&raw_transaction),
            raw_transaction,
        };
        super::Original {
            uuid: uuid::Uuid::new_v4(),
            message,
            submission: signed,
        }
    }

    #[tokio::test]
    async fn prepared_submission_cannot_cross_handoff_without_durable_acknowledgement() {
        use super::{durable_handoff, Response};
        use tokio::sync::mpsc;
        use uuid::Uuid;
        let signed = original(36).await.submission;
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

    struct RetryState {
        chain: Vec<alloy::rpc::types::Block>,
        calls: std::sync::Mutex<Vec<String>>,
        receipt: std::sync::Mutex<serde_json::Value>,
        canonical_receipt: serde_json::Value,
        consumed: std::sync::atomic::AtomicBool,
        processed: std::sync::atomic::AtomicBool,
        reject_send: std::sync::atomic::AtomicBool,
        broadcasts: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    }

    struct RetryRpc {
        api: ethereum_client::EthApi,
        state: std::sync::Arc<RetryState>,
        broadcasts: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        server: tokio::task::JoinHandle<()>,
        _beacon: ethereum_client::beacon_finality::test_fixtures::LocalBeaconFixture,
    }
    impl Drop for RetryRpc {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn retry_rpc() -> anyhow::Result<RetryRpc> {
        use futures::{SinkExt, StreamExt};
        use serde_json::{json, Value};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        };
        use tokio_tungstenite::tungstenite::Message as Frame;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}", listener.local_addr()?);
        let witness_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let witness_url = format!("ws://{}", witness_listener.local_addr()?);
        let tag = uuid::Uuid::new_v4();
        let original = original(36).await;
        let failure = super::super::status_fetcher::tests::reverted_evidence(
            &original.message,
            &original.submission,
        );
        let (chain, receipt, _, _) = super::super::status_fetcher::tests::authenticated_fixture(
            &original.submission,
            failure.receipt,
            tag.as_bytes(),
            1,
            2,
        );
        let (broadcasts, received) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(RetryState {
            chain,
            calls: Mutex::new(vec![]),
            receipt: Mutex::new(Value::Null),
            canonical_receipt: serde_json::to_value(receipt)?,
            consumed: AtomicBool::new(false),
            processed: AtomicBool::new(false),
            reject_send: AtomicBool::new(false),
            broadcasts,
        });
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = async {
                        tokio::select! {
                            accepted = listener.accept() => accepted,
                            accepted = witness_listener.accept() => accepted,
                        }
                    } => {
                        let Ok((socket, _)) = accepted else { break };
                        let state = server_state.clone();
                        clients.spawn(async move {
                            let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { return };
                            while let Some(Ok(Frame::Text(text))) = socket.next().await {
                                let request: Value = serde_json::from_str(&text).unwrap();
                                let method = request["method"].as_str().unwrap();
                                state.calls.lock().unwrap().push(method.to_owned());
                                let params = &request["params"];
                                let result = match method {
                                    "eth_chainId" => json!("0x7a69"),
                                    "eth_getBalance" => json!("0x1000000000000000000"),
                                    "eth_getTransactionReceipt" => state.receipt.lock().unwrap().clone(),
                                    "eth_getBlockReceipts" => json!([state.canonical_receipt]),
                                    "eth_getTransactionByHash" => Value::Null,
                                    "eth_getBlockByNumber" => {
                                        let number = params[0].as_str().unwrap();
                                        let number = if number.starts_with("0x") {
                                            usize::from_str_radix(&number[2..], 16).unwrap()
                                        } else {
                                            1 + usize::from(state.consumed.load(Ordering::SeqCst))
                                        };
                                        json!(state.chain[number])
                                    },
                                    "eth_getBlockByHash" => state.chain.iter()
                                        .find(|block| json!(block.header.hash) == params[0])
                                        .map(|block| json!(block)).unwrap_or(Value::Null),
                                    "eth_getProof" => {
                                        assert_eq!(params[2]["requireCanonical"], json!(true));
                                        let block = state.chain.iter().find(|block|
                                            json!(block.header.hash) == params[2]["blockHash"]
                                        ).unwrap();
                                        let address = serde_json::from_value(params[0].clone()).unwrap();
                                        let (_, proof) = ethereum_client::beacon_finality::test_fixtures::single_account_proof(
                                            address, if block.header.number < 2 { 36 } else { 38 },
                                        );
                                        json!(proof)
                                    },
                                    "eth_sendRawTransaction" => {
                                        let raw = hex::decode(params[0].as_str().unwrap().trim_start_matches("0x")).unwrap();
                                        let hash = alloy::primitives::keccak256(&raw);
                                        state.broadcasts.send(raw).unwrap();
                                        if state.reject_send.load(Ordering::SeqCst) {
                                            let response = json!({"jsonrpc":"2.0", "id":request["id"],
                                                "error":{"code":-32000,"message":"uncertain send"}});
                                            if socket.send(Frame::Text(response.to_string().into())).await.is_err() { break }
                                            continue;
                                        }
                                        json!(hash)
                                    },
                                    _ => panic!("unexpected retry fixture RPC: {method}"),
                                };
                                let response = json!({"jsonrpc":"2.0", "id":request["id"], "result":result});
                                if socket.send(Frame::Text(response.to_string().into())).await.is_err() { break }
                            }
                        });
                    },
                    joined = clients.join_next(), if !clients.is_empty() => {
                        if let Some(Err(error)) = joined { panic!("retry fixture failed: {error}") }
                    },
                }
            }
        });
        let key = format!("0x{}", hex::encode([7; 32]));
        let api = match ethereum_client::EthApi::new(
            &url,
            &format!("{:#x}", alloy::primitives::Address::from([2; 20])),
            Some(&key),
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
        let beacon = ethereum_client::beacon_finality::test_fixtures::LocalBeaconFixture::new(
            31337, state.chain[0].header.hash, Arc::new(move || {
                beacon_state.chain[1 + usize::from(beacon_state.consumed.load(Ordering::SeqCst))].header.clone()
            }),
        ).await?;
        let api = api
            .with_beacon_finality(beacon.config.clone(), &beacon.directory.join("store"))
            .await?
            .with_data_witness(&witness_url, 31337, state.chain[0].header.hash)
            .await?;
        state.calls.lock().unwrap().clear();
        Ok(RetryRpc {
            api,
            state,
            broadcasts: received,
            server,
            _beacon: beacon,
        })
    }

    #[tokio::test]
    async fn missing_original_rebroadcasts_exact_bytes_even_after_competitor_and_send_error(
    ) -> anyhow::Result<()> {
        use super::retry_original;
        use std::sync::atomic::Ordering;
        let mut rpc = retry_rpc().await?;
        let original = original(36).await;
        let guard = rpc.api.reserve_submission().await;
        rpc.state.processed.store(true, Ordering::SeqCst);
        rpc.state.reject_send.store(true, Ordering::SeqCst);
        assert!(retry_original(&rpc.api, &guard, &original).await.is_err());
        assert_eq!(
            rpc.broadcasts.recv().await.unwrap(),
            original.submission.raw_transaction
        );
        let calls = rpc.state.calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .position(|method| method == "eth_getTransactionReceipt")
                .unwrap()
                < calls
                    .iter()
                    .position(|method| method == "eth_getProof")
                    .unwrap(),
            "receipt must precede authenticated account proof"
        );
        assert!(
            calls
                .iter()
                .position(|method| method == "eth_getProof")
                .unwrap()
                < calls
                    .iter()
                    .position(|method| method == "eth_getTransactionByHash")
                    .unwrap()
        );
        assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
        let reconnected = rpc.api.reconnect().await?;
        rpc.state.reject_send.store(false, Ordering::SeqCst);
        retry_original(&reconnected, &guard, &original).await?;
        assert_eq!(
            rpc.broadcasts.recv().await.unwrap(),
            original.submission.raw_transaction
        );
        rpc.state.consumed.store(true, Ordering::SeqCst);
        retry_original(&reconnected, &guard, &original).await?;
        assert!(
            rpc.broadcasts.try_recv().is_err(),
            "consumed original stays held, never replaced"
        );
        assert!(futures::poll!(Box::pin(reconnected.reserve_submission())).is_pending());
        Ok(())
    }

    #[tokio::test]
    async fn restored_originals_retry_in_nonce_order_without_new_handoffs_and_release_only_after_save(
    ) -> anyhow::Result<()> {
        use super::{wait_for_original_completion, MessageSender, MessageSenderIo};
        use std::time::Duration;
        let mut rpc = retry_rpc().await?;
        let first = original(36).await;
        let second = original(37).await;
        let third = original(38).await;
        let (requests, _received) = tokio::sync::mpsc::unbounded_channel();
        let (responses, received) = tokio::sync::mpsc::unbounded_channel();
        let mut io = MessageSenderIo::new(requests, received);
        // Deliberately register the higher nonce first, as unordered restored journals can do.
        assert!(io.register_original(
            second.uuid,
            second.message.clone(),
            second.submission.clone()
        ));
        assert!(io.register_original(first.uuid, first.message.clone(), first.submission.clone()));
        assert!(io.register_original(first.uuid, first.message.clone(), first.submission.clone()));
        assert_eq!(io.original_registrations.borrow().len(), 2);
        let mut changed = first.submission.clone();
        changed.raw_transaction[0] ^= 1;
        assert!(!io.register_original(first.uuid, first.message.clone(), changed));
        let mut ownership = io.original_owners.subscribe();
        let mut registrations = io.original_registrations.subscribe();
        let mut sender = MessageSender::new(rpc.api.clone());
        let guard = rpc.api.reserve_submission().await;
        let waiting = tokio::spawn(async move {
            let result = wait_for_original_completion(
                &mut sender,
                &guard,
                &mut ownership,
                &mut registrations,
                None,
                &responses,
            )
            .await;
            (result, sender.metrics.total_submissions.get())
        });
        for signed in [&first.submission, &second.submission] {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), rpc.broadcasts.recv())
                    .await?
                    .unwrap(),
                signed.raw_transaction
            );
        }
        // A restoration arriving during an earlier original wait must receive retry ownership too.
        assert!(io.register_original(third.uuid, third.message.clone(), third.submission.clone()));
        loop {
            let raw = tokio::time::timeout(Duration::from_secs(5), rpc.broadcasts.recv())
                .await?
                .unwrap();
            if raw == third.submission.raw_transaction {
                break;
            }
            assert!(
                raw == first.submission.raw_transaction || raw == second.submission.raw_transaction
            );
        }
        assert!(
            io.responses.try_recv().is_err(),
            "restore/retry creates neither Prepared nor a second watch"
        );
        assert!(
            futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending(),
            "originals fence unsigned signing"
        );
        io.reconcile_original(first.uuid);
        assert!(
            !waiting.is_finished(),
            "one durable result must not release another original"
        );
        assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
        io.reconcile_original(second.uuid);
        assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
        // Simulate manager acknowledgement only after its last durable disposition save.
        io.reconcile_original(third.uuid);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), waiting).await??,
            (true, 0)
        );
        let _released =
            tokio::time::timeout(Duration::from_secs(5), rpc.api.reserve_submission()).await?;
        Ok(())
    }

    #[tokio::test]
    async fn finalized_original_revert_is_receipt_first_and_still_waits_for_durable_manager_ack(
    ) -> anyhow::Result<()> {
        use super::{retry_original, validate_original};
        use alloy::primitives::B256;
        use serde_json::json;
        use std::sync::atomic::Ordering;
        let mut rpc = retry_rpc().await?;
        let mut original = original(36).await;
        let signed = &original.submission;
        *rpc.state.receipt.lock().unwrap() = json!({
            "type":"0x2", "status":"0x0", "cumulativeGasUsed":"0x5208", "logs":[],
            "logsBloom":format!("0x{}", "00".repeat(256)), "transactionHash":signed.hash,
            "transactionIndex":"0x0", "blockHash":rpc.state.chain[1].header.hash, "blockNumber":"0x1",
            "gasUsed":"0x5208", "effectiveGasPrice":"0x1", "from":signed.sender, "to":signed.contract,
        });
        rpc.state.processed.store(true, Ordering::SeqCst);
        rpc.state.consumed.store(true, Ordering::SeqCst);
        let guard = rpc.api.reserve_submission().await;
        retry_original(&rpc.api, &guard, &original).await?;
        let calls = rpc.state.calls.lock().unwrap().clone();
        assert!(calls
            .iter()
            .any(|method| method == "eth_getTransactionReceipt"));
        assert!(!calls
            .iter()
            .any(|method| method == "eth_getTransactionCount" || method == "eth_call"));
        assert!(rpc.broadcasts.try_recv().is_err());
        assert!(
            futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending(),
            "receipt observation alone cannot unlock the signer"
        );
        for field in ["status", "transactionIndex", "from"] {
            let valid = rpc.state.receipt.lock().unwrap().clone();
            let mut forged = valid.clone();
            forged[field] = match field {
                "status" => json!("0x1"),
                "transactionIndex" => json!("0x1"),
                _ => json!(alloy::primitives::Address::ZERO),
            };
            *rpc.state.receipt.lock().unwrap() = forged;
            assert!(
                retry_original(&rpc.api, &guard, &original).await.is_err(),
                "{field}"
            );
            assert!(futures::poll!(Box::pin(rpc.api.reserve_submission())).is_pending());
            assert!(rpc.broadcasts.try_recv().is_err());
            *rpc.state.receipt.lock().unwrap() = valid;
        }
        original.message.payload.push(9);
        assert!(validate_original(&rpc.api, &original).is_err());
        original.message.payload.pop();
        original.submission.hash = B256::ZERO;
        assert!(validate_original(&rpc.api, &original).is_err());
        Ok(())
    }
}
