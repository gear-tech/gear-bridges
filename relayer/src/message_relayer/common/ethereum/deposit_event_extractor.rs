use ethereum_client::{DepositEventEntry, PollingEthApi};
use primitive_types::H160;
use prometheus::IntCounter;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common,
    message_relayer::{
        common::{
            ethereum::block_listener::ETHEREUM_BLOCK_TIME_APPROX, EthereumBlockNumber,
            EthereumSlotNumber, TxHashWithSlot,
        },
        eth_to_gear::storage::{Storage, UnprocessedBlocks},
    },
};

pub struct DepositEventExtractor {
    eth_api: PollingEthApi,

    erc20_manager_address: H160,

    storage: Arc<dyn Storage>,

    genesis_time: u64,

    metrics: Metrics,
}

impl MeteredService for DepositEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        total_deposits_found: IntCounter = IntCounter::new(
            "deposit_event_extractor_total_deposits_found",
            "Total amount of deposit events discovered",
        ),
    }
}

impl DepositEventExtractor {
    pub fn new(
        eth_api: PollingEthApi,

        erc20_manager_address: H160,
        storage: Arc<dyn Storage>,
        genesis_time: u64,
    ) -> Self {
        Self {
            eth_api,

            erc20_manager_address,
            storage,

            genesis_time,

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
            let UnprocessedBlocks {
                last_block,
                mut unprocessed,
            } = self.storage.block_storage().unprocessed_blocks().await;

            if let Some(last_block) = last_block {
                let latest_finalized_block = match blocks.recv().await {
                    Some(block) => block,
                    None => {
                        log::error!("Failed to fetch missing blocks: channel closed");
                        return;
                    }
                };

                for block in last_block.0 + 1..=latest_finalized_block.0 {
                    unprocessed.push(EthereumBlockNumber(block));
                }
            }

            loop {
                let res = self.run_inner(&sender, &mut blocks, &mut unprocessed).await;
                if let Err(err) = res {
                    attempts += 1;
                    common::retry_backoff(attempts, "Deposit event extractor", &err).await;

                    loop {
                        match self.eth_api.reconnect().await {
                            Ok(api) => {
                                self.eth_api = api;
                                log::info!("Ethereum API reconnected");
                                break;
                            }
                            Err(err) => {
                                log::error!(
                                    "Failed to reconnect to Ethereum: {err}. Retrying in 5s..."
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                } else {
                    log::info!("Block listener connection closed, exiting...");
                    return;
                }
            }
        });

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
        let events = self
            .eth_api
            .fetch_deposit_events(self.erc20_manager_address, block.0)
            .await?;
        let timestamp = self.eth_api.get_block(block.0).await?.header.timestamp;

        let slot_number = EthereumSlotNumber(
            timestamp.saturating_sub(self.genesis_time) / ETHEREUM_BLOCK_TIME_APPROX.as_secs(),
        );

        self.storage
            .block_storage()
            .add_block(slot_number, block, events.iter().map(|ev| ev.tx_hash))
            .await;
        self.storage.save_blocks().await?;
        self.metrics
            .total_deposits_found
            .inc_by(events.len() as u64);

        for DepositEventEntry {
            tx_hash,
            from,
            to,
            token,
            amount,
        } in events
        {
            log::info!(
                "Found deposit event: tx_hash={}, from={}, to={}, token={}, amount={}, slot_number={}",
                hex::encode(tx_hash.0),
                hex::encode(from.0),
                hex::encode(to.0),
                hex::encode(token.0),
                amount,
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
