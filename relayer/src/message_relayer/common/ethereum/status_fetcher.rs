use alloy::providers::{PendingTransactionBuilder, PendingTransactionError, Provider};
use ethereum_client::{EthApi, TxHash};
use futures::{stream::FuturesUnordered, StreamExt};
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    IntCounter, IntGauge,
};
use std::{future::Future, pin::Pin};
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
    Failed(Uuid, PendingTransactionError),
}

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
    // Persist pending requests across reconnects
    let mut pending_requests = std::collections::HashMap::new();

    loop {
        match task_inner(&mut this, &mut channel, &responses, &mut pending_requests).await {
            Ok(_) => break, // Clean exit
            Err(e) => {
                attempts += 1;
                crate::common::retry_backoff(attempts, "Ethereum status fetcher", &e).await;

                // Infinite retry loop for reconnection
                loop {
                    match this.eth_api.reconnect().await.inspect_err(|e| {
                        log::error!("Failed to reconnect to Ethereum: {e}");
                    }) {
                        Ok(eth_api) => {
                            this.eth_api = eth_api;
                            log::info!("Ethereum status fetcher reconnected");
                            break;
                        }
                        Err(_) => {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
    pending_requests: &mut std::collections::HashMap<Uuid, TxHash>,
) -> anyhow::Result<()> {
    type Output = (
        Uuid,
        Result<alloy::rpc::types::TransactionReceipt, alloy::providers::PendingTransactionError>,
    );

    let mut txs = FuturesUnordered::<Pin<Box<dyn Future<Output = Output> + Send>>>::new();

    // Repopulate using current provider
    for (uuid, tx_hash) in pending_requests.iter() {
        let pending =
            PendingTransactionBuilder::new(this.eth_api.raw_provider().root().clone(), *tx_hash);
        let confirmations = this.confirmations;
        let uuid = *uuid;
        txs.push(Box::pin(async move {
            (
                uuid,
                pending
                    .with_required_confirmations(confirmations)
                    .get_receipt()
                    .await,
            )
        }));
    }

    loop {
        tokio::select! {
            message = channel.recv() => {
                let Some(request) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(());
                };

                let Request { tx_uuid, tx_hash, .. } = request;

                this.metrics.pending_tx_count.inc();
                pending_requests.insert(tx_uuid, tx_hash);

                let pending = PendingTransactionBuilder::new(
                    this.eth_api.raw_provider().root().clone(),
                    tx_hash,
                );

                let confirmations = this.confirmations;
                txs.push(Box::pin(async move {
                    (tx_uuid, pending.with_required_confirmations(confirmations).get_receipt().await)
                }));
            }

            Some((uuid, result)) = txs.next() => {
                match result {
                    Ok(receipt) => {
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
                        pending_requests.remove(&uuid);
                        responses.send(Response::Success(uuid, tx_hash))?;
                    }
                    Err(PendingTransactionError::TransportError(e)) => {
                        // Transport error - likely connection lost.
                        // We should return error to trigger reconnect and rebuild of futures.
                        return Err(anyhow::anyhow!("Transport error checking status for {uuid}: {e}"));
                    }
                    Err(e) => {
                        // Other errors (e.g. reverted transaction, or something terminal for THIS tx)
                        this.metrics.total_failed_txs.inc();
                        log::error!("Failed to get transaction {uuid} status: {e}");
                        pending_requests.remove(&uuid);
                        responses.send(Response::Failed(uuid, e))?;
                    }
                }
            }
        }
    }
}
