use crate::common::{self, BASE_RETRY_DELAY, MAX_RETRIES};
use alloy::providers::{PendingTransactionBuilder, PendingTransactionError, Provider};
use ethereum_client::{EthApi, TxHash};
use futures::{stream::FuturesUnordered, StreamExt};
use prometheus::{IntCounter, IntGauge};
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
        let (requests, responses) = (mpsc::unbounded_channel(), mpsc::unbounded_channel());
        tokio::task::spawn(task(self, requests.1, responses.0));

        StatusFetcherIo {
            requests: requests.0,
            responses: responses.1,
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
    let mut txs = FuturesUnordered::new();
    loop {
        tokio::select! {
            message = channel.recv() => {
                let Some(request) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(());
                };

                let Request { tx_uuid, tx_hash, .. } = request;

                this.metrics.pending_tx_count.inc();

                let pending = PendingTransactionBuilder::new(
                    this.eth_api.raw_provider().root().clone(),
                    tx_hash,
                );

                let confirmations = this.confirmations;
                txs.push(async move {
                    Ok((tx_uuid, pending.with_required_confirmations(confirmations).watch().await.map_err(|e| (tx_uuid, e))?))
                });
            }

            Some(tx) = txs.next(), if !txs.is_empty() => {
                match tx {
                    Ok((uuid, tx_hash)) => {
                        this.metrics.pending_tx_count.dec();
                        responses.send(Response::Success(uuid, tx_hash))?;
                    }
                    Err((uuid, e)) => {
                        this.metrics.total_failed_txs.inc();
                        log::error!("Failed to get transaction {uuid} status: {e}");
                        responses.send(Response::Failed(uuid, e))?;
                    }
                }
            }
        }
    }
}
