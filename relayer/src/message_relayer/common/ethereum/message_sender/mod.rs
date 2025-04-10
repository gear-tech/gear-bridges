use std::{
    collections::{btree_map::Entry, BTreeMap},
    time::Duration,
};
use tokio::sync::mpsc::UnboundedReceiver;

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use prometheus::{Gauge, IntGauge};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common,
    message_relayer::common::{AuthoritySetId, MessageInBlock},
};

mod era;
use era::{Era, Metrics as EraMetrics};

use crate::message_relayer::common::RelayedMerkleRoot;

pub struct MessageSender {
    eth_api: EthApi,
    gear_api: GearApi,

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
    pub fn new(eth_api: EthApi, gear_api: GearApi) -> Self {
        Self {
            eth_api,
            gear_api,

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
            let base_delay = Duration::from_secs(1);
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 5;

            loop {
                let res = self.run_inner(&mut messages, &mut merkle_roots).await;
                if let Err(err) = res {
                    attempts += 1;
                    if common::is_transport_error_recoverable(&err) {
                        log::warn!(
                            "Ethereum message sender failed (attempt: {}/{}): {}. Retrying in {:?}",
                            attempts,
                            MAX_ATTEMPTS,
                            err,
                            base_delay * 2u32.pow(attempts)
                        );

                        if attempts >= MAX_ATTEMPTS {
                            log::error!("Ethereum message sender failed too many times: {}", err);
                            break;
                        }

                        tokio::time::sleep(base_delay * 2u32.pow(attempts)).await;
                        match self.eth_api.reconnect().inspect_err(|e| {
                            log::error!("Failed to reconnect to Ethereum: {}", e);
                        }) {
                            Ok(eth_api) => self.eth_api = eth_api,
                            Err(_) => {
                                break;
                            }
                        }
                    } else {
                        log::error!("Ethereum message sender failed: {}", err);
                    }
                }
            }
        });
    }

    async fn run_inner(
        &self,
        messages: &mut UnboundedReceiver<MessageInBlock>,
        merkle_roots: &mut UnboundedReceiver<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

        loop {
            let fee_payer_balance = self.eth_api.get_approx_balance().await?;
            self.metrics.fee_payer_balance.set(fee_payer_balance);

            while let Some(message) = messages.recv().await {
                let authority_set_id = AuthoritySetId(
                    self.gear_api
                        .signed_by_authority_set_id(message.block_hash)
                        .await?,
                );

                match eras.entry(authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_message(message);
                    }
                    Entry::Vacant(entry) => {
                        let mut era = Era::new(authority_set_id, self.era_metrics.clone());
                        era.push_message(message);

                        entry.insert(era);
                    }
                }
            }

            while let Ok(merkle_root) = merkle_roots.try_recv() {
                match eras.entry(merkle_root.authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_merkle_root(merkle_root);
                    }
                    Entry::Vacant(entry) => {
                        let mut era =
                            Era::new(merkle_root.authority_set_id, self.era_metrics.clone());
                        era.push_merkle_root(merkle_root);

                        entry.insert(era);
                    }
                }
            }

            let latest_era = eras.last_key_value().map(|(k, _)| *k);
            let Some(latest_era) = latest_era else {
                continue;
            };

            let mut finalized_eras = vec![];

            for (&era_id, era) in eras.iter_mut() {
                let res = era.process(&self.gear_api, &self.eth_api).await;
                if let Err(err) = res {
                    log::error!("Failed to process era #{}: {}", era_id, err);
                    continue;
                }

                let finalized = era.try_finalize(&self.eth_api, &self.gear_api).await?;

                // Latest era cannot be finalized.
                if finalized && era_id != latest_era {
                    log::info!("Era #{} finalized", era_id);
                    finalized_eras.push(era_id);
                }
            }

            let pending_tx_count: usize = eras.iter().map(|era| era.1.pending_tx_count()).sum();
            self.metrics.pending_tx_count.set(pending_tx_count as i64);

            for finalized in finalized_eras {
                eras.remove(&finalized);
            }
        }
    }
}
