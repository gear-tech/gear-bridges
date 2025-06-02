use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::MessageInBlock,
    },
};
use ethereum_client::{EthApi, TxHash};
use prometheus::{IntCounter, IntGauge};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};
use alloy::providers::{Provider, PendingTransactionBuilder};

pub struct StatusFetcher {
    eth_api: EthApi,
    confirmations: u64,

    metrics: Metrics,
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
    pub fn new(eth_api: EthApi,
    confirmations: u64,) -> Self {
        Self {
            eth_api,
            confirmations,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(
        self,
    ) -> UnboundedSender<(TxHash, MessageInBlock)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::task::spawn(task(self, receiver));

        sender
    }
}

async fn task(
    mut this: StatusFetcher,
    mut channel: UnboundedReceiver<(TxHash, MessageInBlock)>,
) {
    let mut attempts = 0;

    loop {
        match task_inner(&mut this, &mut channel).await {
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
    channel: &mut UnboundedReceiver<(TxHash, MessageInBlock)>,
) -> anyhow::Result<()> {
    while let Some((tx_hash, _message)) = channel.recv().await {
        this.metrics.pending_tx_count.inc();

        let metrics = this.metrics.clone();

        tokio::spawn(get_tx_status(
            this.eth_api.clone(),
            metrics,
            tx_hash,
            this.confirmations,
        ));
    }

    Ok(())
}

async fn get_tx_status(eth_api: EthApi, metrics: Metrics, tx_hash: TxHash, 
    confirmations: u64,) {
    let pending =
        PendingTransactionBuilder::new(eth_api.raw_provider().root().clone(), tx_hash);
    
    if let Err(e) = pending
        .with_required_confirmations(confirmations)
        .watch()
        .await
    {
        metrics.total_failed_txs.inc();
        log::error!("Failed to finalize transaction {tx_hash}: {e:?}");
    } else {
        metrics.pending_tx_count.dec();
        log::info!("Transaction {tx_hash} has {confirmations} confirmation(s)");
    }
}
