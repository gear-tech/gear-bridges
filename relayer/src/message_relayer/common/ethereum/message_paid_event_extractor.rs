use crate::{
    common::{self, BASE_RETRY_DELAY},
    message_relayer::{
        common::{
            ethereum::block_storage::{
                finalized_block, replay_pending_handoffs, BlockCursor, JSONBlockStorage,
            },
            EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot,
        },
        eth_to_gear::storage::Storage,
    },
};
use ethereum_client::PollingEthApi;
use ethereum_common::SECONDS_PER_SLOT;
use primitive_types::H160;
use prometheus::IntCounter;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, Receiver, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MessagePaidEventExtractor {
    eth_api: PollingEthApi,

    storage: Arc<dyn Storage>,
    discovery: Arc<JSONBlockStorage>,

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
        discovery: Arc<JSONBlockStorage>,
    ) -> Self {
        Self {
            storage,
            eth_api,
            discovery,

            bridging_payment_address,

            genesis_time,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self, blocks: Receiver<EthereumBlockNumber>) -> UnboundedReceiver<TxHashWithSlot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(self::task(self, blocks, sender));

        receiver
    }

    pub fn spawn_into(
        self,
        blocks: Receiver<EthereumBlockNumber>,
        sender: UnboundedSender<TxHashWithSlot>,
    ) {
        tokio::task::spawn(self::task(self, blocks, sender));
    }

    async fn run_inner(
        &self,
        sender: &UnboundedSender<TxHashWithSlot>,
        blocks: &mut Receiver<EthereumBlockNumber>,
        missing_blocks: &mut Vec<EthereumBlockNumber>,
    ) -> anyhow::Result<()> {
        replay_pending_handoffs(
            &self.eth_api,
            self.storage.as_ref(),
            self.genesis_time,
            sender,
        )
        .await?;
        while let Some(&block) = missing_blocks.last() {
            self.process_block_events(block, sender).await?;
            missing_blocks.pop();
        }

        while let Some(block) = blocks.recv().await {
            missing_blocks.push(block);
            self.process_block_events(block, sender).await?;
            missing_blocks.pop();
        }

        Ok(())
    }

    async fn process_block_events(
        &self,
        block: EthereumBlockNumber,
        sender: &UnboundedSender<TxHashWithSlot>,
    ) -> anyhow::Result<()> {
        let header = finalized_block(&self.eth_api, block.0).await?;
        let cursor = BlockCursor {
            number: block.0,
            hash: header.header.hash.0.into(),
        };
        if !self.discovery.is_pending(cursor).await? {
            return Ok(());
        }
        let timestamp = header
            .header
            .timestamp
            .checked_sub(self.genesis_time)
            .ok_or_else(|| {
                anyhow::anyhow!("HOLD: Ethereum block timestamp precedes Beacon genesis")
            })?;
        let slot_number = EthereumSlotNumber(timestamp / SECONDS_PER_SLOT);
        let transactions = self
            .eth_api
            .fetch_fee_paid_events_txs_at(self.bridging_payment_address, block.0, cursor.hash)
            .await?;
        super::block_storage::store_extracted_transactions(
            self.storage.as_ref(),
            &self.discovery,
            slot_number,
            cursor,
            transactions.iter().copied(),
        )
        .await?;
        let mut total = 0;
        for tx_hash in transactions {
            if !self
                .storage
                .block_storage()
                .is_transaction_pending(slot_number, tx_hash)
                .await
            {
                continue;
            }
            total += 1;
            log::info!(
                "Found fee paid event: tx_hash={}, slot_number={}",
                hex::encode(tx_hash.0),
                slot_number.0
            );
            sender.send(TxHashWithSlot {
                slot_number,
                tx_hash,
            })?;
        }
        self.metrics.total_paid_messages_found.inc_by(total);
        Ok(())
    }
}

async fn task(
    mut this: MessagePaidEventExtractor,
    mut blocks: Receiver<EthereumBlockNumber>,
    sender: UnboundedSender<TxHashWithSlot>,
) {
    let mut unprocessed = Vec::new();

    let mut attempts: u32 = 0;
    loop {
        let result = this.run_inner(&sender, &mut blocks, &mut unprocessed).await;
        let Err(err) = result else {
            log::info!("Connection to block listener closed, exiting...");
            return;
        };

        if !common::is_transport_error_recoverable(&err) {
            log::error!("Non recoverable paid event extractor error, exiting: {err}");
            return;
        }

        attempts += 1;
        log::error!(
            "Paid event extractor failed with recoverable error (attempt {attempts}): {err}"
        );

        tokio::time::sleep(BASE_RETRY_DELAY * 2u32.pow(attempts.saturating_sub(1).min(6))).await;

        loop {
            match this.eth_api.reconnect().await {
                Ok(eth_api) => {
                    attempts = 0;
                    this.eth_api = eth_api;
                    break;
                }
                Err(err) => {
                    log::error!("Failed to reconnect to Ethereum API: {err}. Retrying...");
                    tokio::time::sleep(BASE_RETRY_DELAY).await;
                }
            }
        }
    }
}
