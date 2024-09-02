use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::mpsc::Receiver,
};

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use prometheus::{Gauge, IntGauge};

use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{AuthoritySetId, MessageInBlock};

pub mod era;
use era::{Era, EraMetrics};

use super::merkle_root_listener::RelayedMerkleRoot;

pub struct MessageSender {
    eth_api: EthApi,
    gear_api: GearApi,

    metrics: Metrics,
    era_metrics: EraMetrics,
}

impl_metered_service! {
    struct Metrics {
        pending_tx_count: IntGauge,
        fee_payer_balance: Gauge
    }
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics
            .get_sources()
            .into_iter()
            .chain(self.era_metrics.get_sources())
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            pending_tx_count: IntGauge::new(
                "message_relayer_message_processor_pending_tx_count",
                "Amount of txs pending finalization on ethereum",
            )?,
            fee_payer_balance: Gauge::new(
                "message_relayer_message_processor_fee_payer_balance",
                "Transaction fee payer balance",
            )?,
        })
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
        self,
        messages: Receiver<MessageInBlock>,
        merkle_roots: Receiver<RelayedMerkleRoot>,
    ) {
        loop {
            let res = self.run_inner(&messages, &merkle_roots).await;
            if let Err(err) = res {
                log::error!("Message relayer failed: {}", err);
            }
        }
    }

    async fn run_inner(
        &self,
        messages: &Receiver<MessageInBlock>,
        merkle_roots: &Receiver<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

        loop {
            let fee_payer_balance = self.eth_api.get_approx_balance().await?;
            self.metrics.fee_payer_balance.set(fee_payer_balance);

            for message in messages.try_iter() {
                let authority_set_id = self
                    .gear_api
                    .signed_by_authority_set_id(message.block_hash)
                    .await?;

                match eras.entry(authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_message(message);
                    }
                    Entry::Vacant(entry) => {
                        let mut era = Era::new(self.era_metrics.clone());
                        era.push_message(message);

                        entry.insert(era);
                    }
                }
            }

            for merkle_root in merkle_roots.try_iter() {
                match eras.entry(merkle_root.authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push_merkle_root(merkle_root);
                    }
                    Entry::Vacant(entry) => {
                        let mut era = Era::new(self.era_metrics.clone());
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

            //let pending_tx_count: usize = eras.iter().map(|era| era.1.pending_txs.len()).sum();
            //self.metrics.pending_tx_count.set(pending_tx_count as i64);

            for finalized in finalized_eras {
                eras.remove(&finalized);
            }
        }
    }
}
