use crate::{
    common,
    message_relayer::{
        common::{EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::storage::{Storage, UnprocessedBlocks},
    },
};
use ethereum_client::PollingEthApi;
use ethereum_common::SECONDS_PER_SLOT;
use primitive_types::H160;
use prometheus::IntCounter;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MessagePaidEventExtractor {
    eth_api: PollingEthApi,

    storage: Arc<dyn Storage>,

    bridging_payment_address: H160,

    genesis_time: u64,

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
        eth_api: PollingEthApi,
        bridging_payment_address: H160,
        storage: Arc<dyn Storage>,
        genesis_time: u64,
    ) -> Self {
        Self {
            storage,
            eth_api,

            bridging_payment_address,

            genesis_time,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(
        self,
        blocks: UnboundedReceiver<EthereumBlockNumber>,
    ) -> UnboundedReceiver<TxHashWithSlot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(self::task(self, blocks, sender));

        receiver
    }

    async fn run_inner(
        &self,
        sender: &UnboundedSender<TxHashWithSlot>,
        blocks: &mut UnboundedReceiver<EthereumBlockNumber>,
        missing_blocks: &mut Vec<EthereumBlockNumber>,
    ) -> anyhow::Result<()> {
        while let Some(block) = missing_blocks.pop() {
            if let Err(e) = self.process_block_events(block, sender).await {
                missing_blocks.push(block);
                return Err(e);
            }
        }

        while let Some(block) = blocks.recv().await {
            if let Err(e) = self.process_block_events(block, sender).await {
                missing_blocks.push(block);
                return Err(e);
            }
        }

        Ok(())
    }

    async fn process_block_events(
        &self,
        block: EthereumBlockNumber,
        sender: &UnboundedSender<TxHashWithSlot>,
    ) -> anyhow::Result<()> {
        let timestamp = self.eth_api.get_block(block.0).await?.header.timestamp;
        let slot_number =
            EthereumSlotNumber(timestamp.saturating_sub(self.genesis_time) / SECONDS_PER_SLOT);

        let txs = self
            .eth_api
            .fetch_fee_paid_events_txs(self.bridging_payment_address, block.0)
            .await?;
        self.storage
            .block_storage()
            .add_block(slot_number, block, txs.iter().cloned())
            .await;
        self.storage.save_blocks().await?;
        self.metrics
            .total_paid_messages_found
            .inc_by(txs.len() as u64);

        for tx_hash in txs {
            log::info!(
                "Found fee paid event: tx_hash={}, slot_number={}",
                hex::encode(tx_hash.0),
                slot_number.0,
            );

            sender.send(TxHashWithSlot {
                slot_number,
                tx_hash,
            })?;
        }

        Ok(())
    }
}

async fn task(
    mut this: MessagePaidEventExtractor,
    mut blocks: UnboundedReceiver<EthereumBlockNumber>,
    sender: UnboundedSender<TxHashWithSlot>,
) {
    let UnprocessedBlocks {
        last_block,
        mut unprocessed,
    } = this.storage.block_storage().unprocessed_blocks().await;

    if let Some(last_block) = last_block {
        let Some(latest_finalized_block) = blocks.recv().await else {
            log::error!("Failed to get latest finalized block: channel closed");
            return;
        };

        for block in last_block.0 + 1..=latest_finalized_block.0 {
            unprocessed.push(EthereumBlockNumber(block));
        }
    }

    let mut attempts = 0;
    loop {
        let result = this.run_inner(&sender, &mut blocks, &mut unprocessed).await;
        let Err(err) = result else {
            log::info!("Connection to block listener closed, exiting...");
            return;
        };

        attempts += 1;
        common::retry_backoff(attempts, "Paid event extractor", &err).await;

        loop {
            match this.eth_api.reconnect().await {
                Ok(eth_api) => {
                    attempts = 0;
                    this.eth_api = eth_api;
                    log::info!("Paid event extractor reconnected");
                    break;
                }
                Err(err) => {
                    log::error!("Failed to reconnect to Ethereum API: {err}. Retrying in 5s...");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            };
        }
    }
}
