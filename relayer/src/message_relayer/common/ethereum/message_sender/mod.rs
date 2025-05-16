use std::collections::{btree_map::Entry, BTreeMap};
use tokio::sync::mpsc::UnboundedReceiver;

use ethereum_client::{EthApi, TxStatus};
use prometheus::{Gauge, IntGauge};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::{AuthoritySetId, MessageInBlock},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use futures::{
    future::{self, Either},
    pin_mut,
};

mod era;
use era::{Era, Metrics as EraMetrics};

use crate::message_relayer::common::RelayedMerkleRoot;

pub struct MessageSender {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,

    metrics: Metrics,
    era_metrics: EraMetrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(self.era_metrics.get_sources())
    }
}

impl_metered_service! {
    struct Metrics {
        pending_tx_count: IntGauge = IntGauge::new(
            "ethereum_message_sender_pending_tx_count",
            "Amount of txs pending finalization on ethereum",
        ),
        fee_payer_balance: Gauge = Gauge::new(
            "ethereum_message_sender_fee_payer_balance",
            "Transaction fee payer balance",
        )
    }
}

impl MessageSender {
    pub fn new(eth_api: EthApi, api_provider: ApiProviderConnection) -> Self {
        Self {
            eth_api,
            api_provider,

            metrics: Metrics::new(),
            era_metrics: EraMetrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    ) {
        tokio::task::spawn(async move {
            let mut attempts = 0;

            loop {
                match run_inner(&mut self, &mut messages).await {
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

                        match self.api_provider.reconnect().await {
                            Ok(()) => {
                                log::info!("Ethereum message sender reconnected");
                            }

                            Err(err) => {
                                log::error!("Ethereum message sender unable to reconnect: {err}");
                                return;
                            }
                        }

                        if common::is_transport_error_recoverable(&e) {
                            match self.eth_api.reconnect().inspect_err(|e| {
                                log::error!("Failed to reconnect to Ethereum: {e}");
                            }) {
                                Ok(eth_api) => self.eth_api = eth_api,
                                Err(_) => {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

async fn run_inner(
    self_: &mut MessageSender,
    messages: &mut UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
) -> anyhow::Result<()> {
    let gear_api = self_.api_provider.client();
    while let Some((message, merkle_root)) = messages.recv().await {
        let tx_hash = era::submit_message(&gear_api, &self_.eth_api, &message.message, merkle_root.block, merkle_root.block_hash).await?;
        log::debug!("tx_hash = {tx_hash}");
        self_.metrics.pending_tx_count.inc();

        let eth_api = self_.eth_api.clone();
        tokio::spawn(async move {
            // wait for 30 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(30 * 60)).await;

            let status = eth_api.get_tx_status(tx_hash).await;
            match status {
                Ok(TxStatus::Pending) => {
                    log::debug!("TxStatus::Pending");
                }

                Ok(TxStatus::Finalized) => {
                    log::info!(
                        "Message at block #{} with nonce {:?} finalized",
                        message.block.0,
                        message.message.nonce_le,
                    );

                    return;
                }

                Ok(TxStatus::Failed) => {
                    log::error!(
                        "Failed to finalize message at block #{} with nonce {:?}",
                        message.block.0,
                        message.message.nonce_le,
                    );

                    return;
                }

                Err(e) => {
                    log::warn!("Unable to get status of the transaction {tx_hash:?}: {e:?}")
                }
            }
        });

        let fee_payer_balance = self_.eth_api.get_approx_balance().await?;
        self_.metrics.fee_payer_balance.set(fee_payer_balance);
    }

    Ok(())
}
