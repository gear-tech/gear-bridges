use crate::common::{self, BASE_RETRY_DELAY, MAX_RETRIES};
use alloy::providers::{
    PendingTransactionBuilder, PendingTransactionError, Provider, RootProvider,
};
use ethereum_client::{EthApi, TxHash};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    IntCounter, IntGauge,
};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

pub struct StatusFetcher {
    eth_api: EthApi,
    confirmations: u64,

    metrics: Metrics,
}

#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub tx_uuid: Uuid,
    pub tx_hash: TxHash,
}

#[derive(Clone, Copy, Debug)]
struct TrackedRequest {
    tx_uuid: Uuid,
    tx_hash: TxHash,
    bridge_nonce: Option<[u8; 32]>,
}

pub enum Response {
    Success(Uuid, TxHash),
    Dropped(Uuid, String),
    Failed(Uuid, String),
}

const TX_VISIBILITY_CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const TX_VISIBILITY_RECHECK_DELAY: Duration = Duration::from_secs(15);
const TX_VISIBILITY_RECHECKS: usize = 3;

enum TxWatchError {
    Receipt(TrackedRequest, PendingTransactionError),
    VisibilityCheck(TrackedRequest),
}

enum Visibility {
    Visible,
    DefinitelyAbsent,
    Inconclusive,
}

#[derive(Debug, PartialEq, Eq)]
enum WatcherFailure {
    ConfirmationTimeout,
    Infrastructure,
}

type TxWatchResult = Result<(TrackedRequest, alloy::rpc::types::TransactionReceipt), TxWatchError>;
type TxWatch = BoxFuture<'static, TxWatchResult>;
type VisibilityCheck = BoxFuture<'static, (TrackedRequest, Visibility, EthApi)>;
type Reconciliation = BoxFuture<'static, (TrackedRequest, Result<bool, String>, EthApi)>;

pub struct StatusFetcherIo {
    requests: UnboundedSender<TrackedRequest>,
    responses: UnboundedReceiver<Response>,
}

impl StatusFetcherIo {
    pub fn send_request(&self, tx_uuid: Uuid, tx_hash: TxHash) -> bool {
        self.requests
            .send(TrackedRequest {
                tx_uuid,
                tx_hash,
                bridge_nonce: None,
            })
            .is_ok()
    }

    pub fn send_request_with_bridge_nonce(
        &self,
        tx_uuid: Uuid,
        tx_hash: TxHash,
        bridge_nonce: [u8; 32],
    ) -> bool {
        self.requests
            .send(TrackedRequest {
                tx_uuid,
                tx_hash,
                bridge_nonce: Some(bridge_nonce),
            })
            .is_ok()
    }

    pub async fn recv_message(&mut self) -> Option<Response> {
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
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let (responses_tx, responses_rx) = mpsc::unbounded_channel();
        tokio::task::spawn(task(self, requests_rx, responses_tx));

        StatusFetcherIo {
            requests: requests_tx,
            responses: responses_rx,
        }
    }
}

async fn task(
    mut this: StatusFetcher,
    mut channel: UnboundedReceiver<TrackedRequest>,
    responses: UnboundedSender<Response>,
) {
    let mut attempts = 0;

    loop {
        match task_inner(&mut this, &mut channel, &responses).await {
            Ok(_) => break,
            Err(e) => {
                attempts += 1;
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                log::error!(
                "Ethereum message sender failed (attempt: {attempts}/{MAX_RETRIES}): {e}. Retrying in {delay:?}",
            );
                if attempts >= MAX_RETRIES {
                    log::error!("Maximum attempts reached, exiting...");
                    break;
                }

                tokio::time::sleep(delay).await;

                if common::is_transport_error_recoverable(&e) {
                    match this.eth_api.reconnect().await.inspect_err(|e| {
                        log::error!("Failed to reconnect to Ethereum: {e}");
                    }) {
                        Ok(eth_api) => this.eth_api = eth_api,
                        Err(_) => {
                            break;
                        }
                    }
                }
            }
        }
    }
}

async fn task_inner(
    this: &mut StatusFetcher,
    channel: &mut UnboundedReceiver<TrackedRequest>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<()> {
    let mut txs: FuturesUnordered<TxWatch> = FuturesUnordered::new();
    let mut visibility_checks: FuturesUnordered<VisibilityCheck> = FuturesUnordered::new();
    let mut reconciliations: FuturesUnordered<Reconciliation> = FuturesUnordered::new();

    loop {
        tokio::select! {
            message = channel.recv() => {
                let Some(request) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(());
                };

                this.metrics.pending_tx_count.inc();
                watch_with_api(&this.eth_api, &mut txs, request, this.confirmations);
            }

            Some(tx) = txs.next(), if !txs.is_empty() => {
                match tx {
                    Ok((request, receipt)) => {
                        let tx_hash = receipt.transaction_hash;
                        let gas_used = receipt.gas_used;

                        this.metrics.total_gas_used.inc_by(gas_used);
                        this.metrics.last_gas_used.set(gas_used);

                        if this.metrics.min_gas_used.get() == 0 || gas_used < this.metrics.min_gas_used.get() {
                            this.metrics.min_gas_used.set(gas_used);
                        }

                        if gas_used > this.metrics.max_gas_used.get() {
                            this.metrics.max_gas_used.set(gas_used);
                        }

                        if receipt.status() {
                            this.metrics.pending_tx_count.dec();
                            responses.send(Response::Success(request.tx_uuid, tx_hash))?;
                        } else {
                            this.metrics.total_failed_txs.inc();
                            // A different submission can process the bridge message while this
                            // transaction is pending. Reconcile against finalized bridge state;
                            // an unprocessed message follows the bounded dropped-transaction retry.
                            if request.bridge_nonce.is_some() {
                                reconcile_reverted_receipt(
                                    this.eth_api.clone(),
                                    &mut reconciliations,
                                    request,
                                    Duration::ZERO,
                                );
                            } else {
                                // Preserve the legacy two-argument API for callers that do not
                                // have the bridge nonce; they still receive a bounded retry.
                                this.metrics.pending_tx_count.dec();
                                responses.send(reverted_receipt_response(request, false))?;
                            }
                        }
                    }
                    Err(TxWatchError::VisibilityCheck(request)) => {
                        log::warn!(
                            "Timed out while polling transaction {}. Checking whether it is still visible",
                            request.tx_hash
                        );
                        start_visibility_check(this.eth_api.clone(), &mut visibility_checks, request);
                    }
                    Err(TxWatchError::Receipt(request, error)) => {
                        // PendingTransactionError describes watcher/transport infrastructure, not
                        // EVM execution. Do not turn it into a terminal bridge-message failure.
                        match classify_watcher_failure(&error) {
                            WatcherFailure::ConfirmationTimeout => log::warn!(
                                "Timed out while polling transaction {}: {error}. Checking whether it is still visible",
                                request.tx_hash
                            ),
                            WatcherFailure::Infrastructure => log::warn!(
                                "Transaction watcher infrastructure failed for {}: {error}. Checking visibility before continuing",
                                request.tx_hash
                            ),
                        }
                        start_visibility_check(this.eth_api.clone(), &mut visibility_checks, request);
                    }
                }
            }

            Some((request, visibility, eth_api)) = visibility_checks.next(), if !visibility_checks.is_empty() => {
                match visibility {
                    Visibility::Visible => {
                        log::info!(
                            "Transaction {} is still visible; continuing to watch",
                            request.tx_hash
                        );
                        watch_with_api(&eth_api, &mut txs, request, this.confirmations);
                    }
                    Visibility::DefinitelyAbsent => {
                        this.metrics.pending_tx_count.dec();
                        this.metrics.total_failed_txs.inc();
                        let error = format!(
                            "Ethereum transaction {} disappeared before receiving confirmations",
                            request.tx_hash
                        );
                        log::error!("{error}");
                        responses.send(Response::Dropped(request.tx_uuid, error))?;
                    }
                    Visibility::Inconclusive => {
                        log::warn!(
                            "Transaction {} visibility remained inconclusive; continuing to watch without resubmitting",
                            request.tx_hash
                        );
                        watch_with_api(&eth_api, &mut txs, request, this.confirmations);
                    }
                }
            }

            Some((request, processed, eth_api)) = reconciliations.next(), if !reconciliations.is_empty() => {
                match processed {
                    Ok(true) => {
                        this.metrics.pending_tx_count.dec();
                        log::info!(
                            "Ethereum transaction {} reverted, but bridge message nonce {} is already processed",
                            request.tx_hash,
                            hex::encode(request.bridge_nonce.expect("reconciliation requires bridge nonce")),
                        );
                        responses.send(reverted_receipt_response(request, true))?;
                    }
                    Ok(false) => {
                        this.metrics.pending_tx_count.dec();
                        let response = reverted_receipt_response(request, false);
                        if let Response::Dropped(_, ref error) = response {
                            log::error!("{error}");
                        }
                        responses.send(response)?;
                    }
                    Err(error) => {
                        log::warn!(
                            "Failed to reconcile reverted transaction {} against bridge message nonce {}: {error}. Retrying",
                            request.tx_hash,
                            hex::encode(request.bridge_nonce.expect("reconciliation requires bridge nonce")),
                        );
                        reconcile_reverted_receipt(
                            eth_api,
                            &mut reconciliations,
                            request,
                            BASE_RETRY_DELAY,
                        );
                    }
                }
            }
        }
    }
}

fn watch_with_api(
    eth_api: &EthApi,
    txs: &mut FuturesUnordered<TxWatch>,
    request: TrackedRequest,
    confirmations: u64,
) {
    txs.push(watch_tx(
        eth_api.raw_provider().root().clone(),
        request,
        confirmations,
    ));
}

fn start_visibility_check(
    eth_api: EthApi,
    checks: &mut FuturesUnordered<VisibilityCheck>,
    request: TrackedRequest,
) {
    checks.push(check_transaction_visibility(eth_api, request));
}

fn check_transaction_visibility(mut eth_api: EthApi, request: TrackedRequest) -> VisibilityCheck {
    Box::pin(async move {
        let mut definitely_absent = true;

        for attempt in 1..=TX_VISIBILITY_RECHECKS {
            let (transaction, receipt) = tokio::join!(
                eth_api
                    .raw_provider()
                    .get_transaction_by_hash(request.tx_hash),
                eth_api
                    .raw_provider()
                    .get_transaction_receipt(request.tx_hash),
            );

            match (transaction, receipt) {
                (Ok(Some(_)), _) | (_, Ok(Some(_))) => {
                    return (request, Visibility::Visible, eth_api);
                }
                (Ok(None), Ok(None)) => {
                    log::warn!(
                        "Transaction {} was absent during visibility check {attempt}/{TX_VISIBILITY_RECHECKS}",
                        request.tx_hash
                    );
                }
                (transaction, receipt) => {
                    definitely_absent = false;
                    log::warn!(
                        "Transaction {} visibility check {attempt}/{TX_VISIBILITY_RECHECKS} was inconclusive: transaction={transaction:?}, receipt={receipt:?}",
                        request.tx_hash
                    );
                }
            }

            if attempt < TX_VISIBILITY_RECHECKS {
                match eth_api.reconnect().await {
                    Ok(reconnected) => eth_api = reconnected,
                    Err(error) => log::warn!(
                        "Failed to reconnect while checking transaction {}: {error}",
                        request.tx_hash
                    ),
                }
                tokio::time::sleep(TX_VISIBILITY_RECHECK_DELAY).await;
            }
        }

        let visibility = if definitely_absent {
            Visibility::DefinitelyAbsent
        } else {
            Visibility::Inconclusive
        };
        (request, visibility, eth_api)
    })
}

fn reconcile_reverted_receipt(
    mut eth_api: EthApi,
    reconciliations: &mut FuturesUnordered<Reconciliation>,
    request: TrackedRequest,
    delay: Duration,
) {
    reconciliations.push(Box::pin(async move {
        tokio::time::sleep(delay).await;
        let bridge_nonce = request
            .bridge_nonce
            .expect("reconciliation requires bridge nonce");
        let processed = eth_api
            .is_message_processed(bridge_nonce)
            .await
            .map_err(|error| error.to_string());

        if processed.is_err() {
            match eth_api.reconnect().await {
                Ok(reconnected) => eth_api = reconnected,
                Err(error) => log::warn!(
                    "Failed to reconnect after bridge-state reconciliation error for transaction {}: {error}",
                    request.tx_hash
                ),
            }
        }

        (request, processed, eth_api)
    }));
}

fn reverted_receipt_response(request: TrackedRequest, processed: bool) -> Response {
    if processed {
        Response::Success(request.tx_uuid, request.tx_hash)
    } else {
        Response::Dropped(
            request.tx_uuid,
            format!(
                "Ethereum transaction {} reverted while the bridge message remained unprocessed",
                request.tx_hash
            ),
        )
    }
}

fn classify_watcher_failure(error: &PendingTransactionError) -> WatcherFailure {
    match error {
        PendingTransactionError::TxWatcher(_) => WatcherFailure::ConfirmationTimeout,
        PendingTransactionError::FailedToRegister
        | PendingTransactionError::TransportError(_)
        | PendingTransactionError::Recv(_) => WatcherFailure::Infrastructure,
    }
}

fn watch_tx(provider: RootProvider, request: TrackedRequest, confirmations: u64) -> TxWatch {
    Box::pin(async move {
        let pending = PendingTransactionBuilder::new(provider, request.tx_hash)
            .with_required_confirmations(confirmations);

        match tokio::time::timeout(TX_VISIBILITY_CHECK_INTERVAL, pending.get_receipt()).await {
            Ok(Ok(receipt)) => Ok((request, receipt)),
            Ok(Err(error)) => Err(TxWatchError::Receipt(request, error)),
            Err(_) => Err(TxWatchError::VisibilityCheck(request)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::providers::WatchTxError;

    #[test]
    fn classifies_watcher_failures_without_treating_infrastructure_as_execution_failure() {
        assert_eq!(
            classify_watcher_failure(&PendingTransactionError::FailedToRegister),
            WatcherFailure::Infrastructure
        );
        assert_eq!(
            classify_watcher_failure(&PendingTransactionError::TxWatcher(WatchTxError::Timeout)),
            WatcherFailure::ConfirmationTimeout
        );
    }

    #[test]
    fn reverted_receipt_is_success_or_bounded_retry_based_on_bridge_state() {
        let request = TrackedRequest {
            tx_uuid: Uuid::new_v4(),
            tx_hash: TxHash::from([9; 32]),
            bridge_nonce: Some([7; 32]),
        };

        assert!(matches!(
            reverted_receipt_response(request, true),
            Response::Success(uuid, hash) if uuid == request.tx_uuid && hash == request.tx_hash
        ));
        assert!(matches!(
            reverted_receipt_response(request, false),
            Response::Dropped(uuid, _) if uuid == request.tx_uuid
        ));
    }
}
