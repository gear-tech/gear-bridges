use prometheus::IntCounter;
use sails_rs::H160;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, FeePaidEntry};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
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

    pub async fn run(
        mut self,
        mut blocks: UnboundedReceiver<EthereumBlockNumber>,
    ) -> UnboundedReceiver<TxHashWithSlot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            let mut attempts = 0;

            loop {
                let res = self.run_inner(&sender, &mut blocks).await;
                if let Err(err) = res {
                    attempts += 1;
                    log::error!(
                        "Deposit event extractor failed (attempt {attempts}/{MAX_RETRIES}): {err}"
                    );

                    if attempts >= MAX_RETRIES {
                        log::error!("Max attempts reached, exiting...");
                        break;
                    }

                    tokio::time::sleep(BASE_RETRY_DELAY * 2u32.pow(attempts - 1)).await;

                    if common::is_transport_error_recoverable(&err) {
                        self.eth_api = match self.eth_api.reconnect().await {
                            Ok(eth_api) => eth_api,
                            Err(err) => {
                                log::error!("Failed to reconnect to Ethereum API: {err}");
                                break;
                            }
                        };
                    }
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &UnboundedSender<TxHashWithSlot>,
        blocks: &mut UnboundedReceiver<EthereumBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            while let Some(block) = blocks.recv().await {
                self.process_block_events(block, sender).await?;
            }
        }
    }

    async fn process_block_events(
        &self,
        block: EthereumBlockNumber,
        sender: &UnboundedSender<TxHashWithSlot>,
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
