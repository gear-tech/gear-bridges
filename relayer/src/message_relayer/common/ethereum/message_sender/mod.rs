use std::collections::{btree_map::Entry, BTreeMap};
use tokio::sync::mpsc::UnboundedReceiver;

use ethereum_client::EthApi;
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
        mut messages: UnboundedReceiver<MessageInBlock>,
        mut merkle_roots: UnboundedReceiver<RelayedMerkleRoot>,
    ) {
        tokio::task::spawn(async move {
            let mut attempts = 0;

            loop {
                match run_inner(&mut self, &mut messages, &mut merkle_roots).await {
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
    messages: &mut UnboundedReceiver<MessageInBlock>,
    merkle_roots: &mut UnboundedReceiver<RelayedMerkleRoot>,
) -> anyhow::Result<()> {
    let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();
    let gear_api = self_.api_provider.client();
    loop {
        let fee_payer_balance = self_.eth_api.get_approx_balance().await?;
        self_.metrics.fee_payer_balance.set(fee_payer_balance);

        let recv_messages = messages.recv();
        pin_mut!(recv_messages);

        let recv_merkle_roots = merkle_roots.recv();
        pin_mut!(recv_merkle_roots);

        match future::select(recv_messages, recv_merkle_roots).await {
            Either::Left((None, _)) => {
                log::info!("Channel with messages closed. Exiting");
                return Ok(());
            }

            Either::Right((None, _)) => {
                log::info!("Channel with merkle roots closed. Exiting");
                return Ok(());
            }

            Either::Left((Some(message), _)) => {
                let authority_set_id = AuthoritySetId(
                    gear_api
                        .signed_by_authority_set_id(message.block_hash)
                        .await?,
                );

                match eras.entry(authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_message(message);
                    }
                    Entry::Vacant(entry) => {
                        let mut era = Era::new(authority_set_id, self_.era_metrics.clone());
                        era.push_message(message);

                        entry.insert(era);
                    }
                }
            }

            Either::Right((Some(merkle_root), _)) => {
                match eras.entry(merkle_root.authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_merkle_root(merkle_root);
                    }
                    Entry::Vacant(entry) => {
                        let mut era =
                            Era::new(merkle_root.authority_set_id, self_.era_metrics.clone());
                        era.push_merkle_root(merkle_root);

                        entry.insert(era);
                    }
                }
            }
        }

        let latest_era = eras.last_key_value().map(|(k, _)| *k);
        let Some(latest_era) = latest_era else {
            continue;
        };

        let mut finalized_eras = vec![];

        for (&era_id, era) in eras.iter_mut() {
            let res = era.process(&gear_api, &self_.eth_api).await;
            if let Err(err) = res {
                log::error!("Failed to process era #{era_id}: {err}");
                continue;
            }

            let finalized = era.try_finalize(&self_.eth_api, &gear_api).await?;

            // Latest era cannot be finalized.
            if finalized && era_id != latest_era {
                log::info!("Era #{era_id} finalized");
                finalized_eras.push(era_id);
            }
        }

        let pending_tx_count: usize = eras.iter().map(|era| era.1.pending_tx_count()).sum();
        self_.metrics.pending_tx_count.set(pending_tx_count as i64);

        for finalized in finalized_eras {
            eras.remove(&finalized);
        }
    }
}
