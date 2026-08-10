use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    rpc,
};
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

pub struct Request {
    pub tx_uuid: Uuid,
    pub tx_hash: TxHash,
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
    Receipt(Uuid, TxHash, PendingTransactionError),
    VisibilityCheck(Uuid, TxHash),
}

type TxWatchResult = Result<(Uuid, alloy::rpc::types::TransactionReceipt), TxWatchError>;
type TxWatch = BoxFuture<'static, TxWatchResult>;

pub struct StatusFetcherIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl StatusFetcherIo {
    pub fn send_request(&self, tx_uuid: Uuid, tx_hash: TxHash) -> bool {
        let request = Request { tx_uuid, tx_hash };
        self.requests.send(request).is_ok()
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
    mut channel: UnboundedReceiver<Request>,
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
    channel: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<()> {
    let mut txs: FuturesUnordered<TxWatch> = FuturesUnordered::new();
    loop {
        tokio::select! {
            message = channel.recv() => {
                let Some(request) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(());
                };

                let Request { tx_uuid, tx_hash, .. } = request;

                this.metrics.pending_tx_count.inc();

                txs.push(watch_tx(
                    this.eth_api.raw_provider().root().clone(),
                    tx_uuid,
                    tx_hash,
                    this.confirmations,
                ));
            }

            Some(tx) = txs.next(), if !txs.is_empty() => {
                match tx {
                    Ok((uuid, receipt)) => {
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

                        this.metrics.pending_tx_count.dec();
                        if receipt.status() {
                            responses.send(Response::Success(uuid, tx_hash))?;
                        } else {
                            this.metrics.total_failed_txs.inc();
                            let error = format!("Ethereum transaction {tx_hash} reverted");
                            log::error!("{error}");
                            responses.send(Response::Failed(uuid, error))?;
                        }
                    }
                    Err(TxWatchError::VisibilityCheck(uuid, tx_hash)) => {
                        check_transaction_visibility(this, &mut txs, responses, uuid, tx_hash).await?;
                    }
                    Err(TxWatchError::Receipt(uuid, tx_hash, e))
                        if is_timeout_error(&e) =>
                    {
                        log::warn!(
                            "Timed out while polling transaction {tx_hash}: {e}. Checking whether it is still visible"
                        );
                        check_transaction_visibility(this, &mut txs, responses, uuid, tx_hash).await?;
                    }
                    Err(TxWatchError::Receipt(uuid, tx_hash, e))
                        if rpc::is_recoverable_error_text(&e) =>
                    {
                        log::warn!("Recoverable error while polling transaction {tx_hash}: {e}. Reconnecting and continuing to watch");
                        tokio::time::sleep(BASE_RETRY_DELAY).await;
                        match this.eth_api.reconnect().await {
                            Ok(eth_api) => this.eth_api = eth_api,
                            Err(reconnect_error) => log::warn!(
                                "Failed to reconnect while watching transaction {tx_hash}: {reconnect_error}"
                            ),
                        }
                        rewatch(this, &mut txs, uuid, tx_hash);
                    }
                    Err(TxWatchError::Receipt(uuid, tx_hash, e)) => {
                        this.metrics.pending_tx_count.dec();
                        this.metrics.total_failed_txs.inc();
                        let error = format!("Failed to get transaction {tx_hash} status: {e}");
                        log::error!("{error}");
                        responses.send(Response::Failed(uuid, error))?;
                    }
                }
            }
        }
    }
}

fn rewatch(this: &StatusFetcher, txs: &mut FuturesUnordered<TxWatch>, uuid: Uuid, tx_hash: TxHash) {
    txs.push(watch_tx(
        this.eth_api.raw_provider().root().clone(),
        uuid,
        tx_hash,
        this.confirmations,
    ));
}

async fn check_transaction_visibility(
    this: &mut StatusFetcher,
    txs: &mut FuturesUnordered<TxWatch>,
    responses: &UnboundedSender<Response>,
    uuid: Uuid,
    tx_hash: TxHash,
) -> anyhow::Result<()> {
    let mut definitely_absent = true;

    for attempt in 1..=TX_VISIBILITY_RECHECKS {
        let transaction = this
            .eth_api
            .raw_provider()
            .get_transaction_by_hash(tx_hash)
            .await;
        let receipt = this
            .eth_api
            .raw_provider()
            .get_transaction_receipt(tx_hash)
            .await;

        match (transaction, receipt) {
            (Ok(Some(_)), _) | (_, Ok(Some(_))) => {
                log::info!("Transaction {tx_hash} is still visible; continuing to watch");
                rewatch(this, txs, uuid, tx_hash);
                return Ok(());
            }
            (Ok(None), Ok(None)) => {
                log::warn!(
                    "Transaction {tx_hash} was absent during visibility check {attempt}/{TX_VISIBILITY_RECHECKS}"
                );
                if attempt < TX_VISIBILITY_RECHECKS {
                    match this.eth_api.reconnect().await {
                        Ok(eth_api) => this.eth_api = eth_api,
                        Err(reconnect_error) => log::warn!(
                            "Failed to reconnect between visibility checks for transaction {tx_hash}: {reconnect_error}"
                        ),
                    }
                }
            }
            (transaction, receipt) => {
                definitely_absent = false;
                log::warn!(
                    "Transaction {tx_hash} visibility check {attempt}/{TX_VISIBILITY_RECHECKS} was inconclusive: transaction={transaction:?}, receipt={receipt:?}"
                );
                match this.eth_api.reconnect().await {
                    Ok(eth_api) => this.eth_api = eth_api,
                    Err(reconnect_error) => log::warn!(
                        "Failed to reconnect while checking transaction {tx_hash}: {reconnect_error}"
                    ),
                }
            }
        }

        if attempt < TX_VISIBILITY_RECHECKS {
            tokio::time::sleep(TX_VISIBILITY_RECHECK_DELAY).await;
        }
    }

    if definitely_absent {
        this.metrics.pending_tx_count.dec();
        this.metrics.total_failed_txs.inc();
        let error =
            format!("Ethereum transaction {tx_hash} disappeared before receiving confirmations");
        log::error!("{error}");
        responses.send(Response::Dropped(uuid, error))?;
    } else {
        log::warn!(
            "Transaction {tx_hash} visibility remained inconclusive; continuing to watch without resubmitting"
        );
        rewatch(this, txs, uuid, tx_hash);
    }

    Ok(())
}

fn is_timeout_error(error: &PendingTransactionError) -> bool {
    let error = error.to_string().to_ascii_lowercase();
    error.contains("timeout") || error.contains("timed out")
}

fn watch_tx(provider: RootProvider, tx_uuid: Uuid, tx_hash: TxHash, confirmations: u64) -> TxWatch {
    Box::pin(async move {
        let pending = PendingTransactionBuilder::new(provider, tx_hash)
            .with_required_confirmations(confirmations);

        match tokio::time::timeout(TX_VISIBILITY_CHECK_INTERVAL, pending.get_receipt()).await {
            Ok(Ok(receipt)) => Ok((tx_uuid, receipt)),
            Ok(Err(error)) => Err(TxWatchError::Receipt(tx_uuid, tx_hash, error)),
            Err(_) => Err(TxWatchError::VisibilityCheck(tx_uuid, tx_hash)),
        }
    })
}
