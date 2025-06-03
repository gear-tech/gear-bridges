use crate::{
    common::{self, BASE_RETRY_DELAY},
    message_relayer::common::{Data, MessageInBlock},
};
use ethereum_client::{EthApi, TxHash};
use prometheus::Gauge;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MessageSender {
    max_retries: u32,
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
    }
}

impl MessageSender {
    pub fn new(max_retries: u32, eth_api: EthApi) -> Self {
        Self {
            max_retries,
            eth_api,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(
        self,
        channel_message_data: UnboundedReceiver<Data>,
        channel_tx_data: UnboundedSender<(TxHash, MessageInBlock)>,
    ) {
        tokio::task::spawn(task(self, channel_message_data, channel_tx_data));
    }
}

async fn task(
    mut this: MessageSender,
    mut channel_message_data: UnboundedReceiver<Data>,
    channel_tx_data: UnboundedSender<(TxHash, MessageInBlock)>,
) {
    if let Ok(fee_payer_balance) = this.eth_api.get_approx_balance().await {
        this.metrics.fee_payer_balance.set(fee_payer_balance);
    }

    let mut attempts = 0;

    loop {
        match task_inner(&mut this, &mut channel_message_data, &channel_tx_data).await {
            Ok(_) => break,
            Err(e) => {
                attempts += 1;
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                log::error!(
                "Ethereum message sender failed (attempt: {attempts}/{}): {e}. Retrying in {delay:?}",
                this.max_retries,
            );
                if attempts >= this.max_retries {
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
    this: &mut MessageSender,
    channel_message_data: &mut UnboundedReceiver<Data>,
    channel_tx_data: &UnboundedSender<(TxHash, MessageInBlock)>,
) -> anyhow::Result<()> {
    while let Some(data) = channel_message_data.recv().await {
        let Data {
            message,
            relayed_root,
            proof,
        } = data;

        let tx_hash = this
            .eth_api
            .provide_content_message(
                relayed_root.block.0,
                proof.num_leaves as u32,
                proof.leaf_index as u32,
                message.message.nonce_le,
                message.message.source,
                message.message.destination,
                message.message.payload.to_vec(),
                proof.proof,
            )
            .await?;

        log::info!(
            "Message with nonce {} relaying started: tx_hash = {tx_hash}",
            hex::encode(message.message.nonce_le)
        );

        channel_tx_data.send((tx_hash, message))?;

        let fee_payer_balance = this.eth_api.get_approx_balance().await?;
        this.metrics.fee_payer_balance.set(fee_payer_balance);
    }

    Ok(())
}
