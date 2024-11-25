use std::sync::mpsc::{channel, Receiver, Sender};

use futures::executor::block_on;
use prometheus::IntCounter;
use sails_rs::H160;

use ethereum_client::{EthApi, FeePaidEntry};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    ethereum_beacon_client::BeaconClient,
    message_relayer::common::{EthereumBlockNumber, TxHashWithSlot},
};

use super::find_slot_by_block_number;

pub struct MessagePaidEventExtractor {
    eth_api: EthApi,
    beacon_client: BeaconClient,

    bridging_payment_address: H160,

    metrics: Metrics,
}

impl MeteredService for MessagePaidEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        total_paid_messages_found: IntCounter = IntCounter::new(
            "message_paid_event_extractor_total_paid_messages_found",
            "Total amount of paid messages discovered",
        ),
    }
}

impl MessagePaidEventExtractor {
    pub fn new(
        eth_api: EthApi,
        beacon_client: BeaconClient,
        bridging_payment_address: H160,
    ) -> Self {
        Self {
            eth_api,
            beacon_client,

            bridging_payment_address,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<EthereumBlockNumber>) -> Receiver<TxHashWithSlot> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Deposit event extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<TxHashWithSlot>,
        blocks: &Receiver<EthereumBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: EthereumBlockNumber,
        sender: &Sender<TxHashWithSlot>,
    ) -> anyhow::Result<()> {
        let events = self
            .eth_api
            .fetch_fee_paid_events(self.bridging_payment_address, block.0)
            .await?;

        if events.is_empty() {
            return Ok(());
        }

        let slot_number =
            find_slot_by_block_number(&self.eth_api, &self.beacon_client, block).await?;

        self.metrics
            .total_paid_messages_found
            .inc_by(events.len() as u64);

        for ev in &events {
            log::info!(
                "Found fee paid event: tx_hash={}, slot_number={}",
                hex::encode(ev.tx_hash.0),
                slot_number.0,
            );
        }

        for FeePaidEntry { tx_hash } in events {
            sender.send(TxHashWithSlot {
                slot_number,
                tx_hash,
            })?;
        }

        Ok(())
    }
}
