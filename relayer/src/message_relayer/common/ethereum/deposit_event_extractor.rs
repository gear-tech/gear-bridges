use std::{collections::VecDeque, sync::Arc};

use prometheus::IntCounter;
use sails_rs::H160;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{DepositEventEntry, EthApi};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::{EthereumBlockNumber, TxHashWithSlot},
        eth_to_gear::storage::Storage,
    },
};

use super::find_slot_by_block_number;

pub struct DepositEventExtractor {
    eth_api: EthApi,
    beacon_client: BeaconClient,

    erc20_manager_address: H160,

    storage: Arc<dyn Storage>,

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
        eth_api: EthApi,
        beacon_client: BeaconClient,
        erc20_manager_address: H160,
        storage: Arc<dyn Storage>,
    ) -> Self {
        Self {
            eth_api,
            beacon_client,

            erc20_manager_address,
            storage,

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
            let mut unprocessed = self.storage.block_storage().unprocessed_blocks().await;

            if let Some(last_block) = unprocessed.last_block {
                let latest_finalized_block = match self.eth_api.finalized_block_number().await {
                    Ok(block) => block,
                    Err(err) => {
                        log::error!("Failed to fetch missing blocks: {err:?}");
                        return;
                    }
                };

                for block in last_block.0 + 1..=latest_finalized_block {
                    unprocessed.unprocessed.push(EthereumBlockNumber(block));
                }
            }
            loop {
                let res = self
                    .run_inner(&sender, &mut blocks, &mut unprocessed.unprocessed)
                    .await;
                if let Err(err) = res {
                    attempts += 1;
                    let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);

                    log::error!(
                        "Deposit event extractor failed (attempt {attempts}/{MAX_RETRIES}): {err}. Retrying in {delay:?}"
                    );
                    if attempts >= MAX_RETRIES {
                        log::error!("Maximum attempts reached, exiting...");
                        break;
                    }
                    tokio::time::sleep(delay).await;
                    if common::is_transport_error_recoverable(&err) {
                        self.eth_api = match self.eth_api.reconnect().await {
                            Ok(api) => api,
                            Err(err) => {
                                log::error!("Failed to reconnect to Ethereum: {err}");
                                break;
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
            self.process_block_events(block, sender).await?;
        }
        while let Some(block) = blocks.recv().await {
            self.process_block_events(block, sender).await?;
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

        let slot_number =
            find_slot_by_block_number(&self.eth_api, &self.beacon_client, block).await?;

        self.storage
            .block_storage()
            .add_block(slot_number, block, events.iter().map(|ev| ev.tx_hash))
            .await;

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
