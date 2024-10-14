use std::sync::mpsc::{channel, Receiver, Sender};

use alloy::providers::Provider;
use alloy_eips::BlockNumberOrTag;
use anyhow::anyhow;
use futures::executor::block_on;
use prometheus::IntCounter;
use sails_rs::H160;

use ethereum_client::{DepositEventEntry, EthApi};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    ethereum_beacon_client::BeaconClient,
    message_relayer::common::{ERC20DepositTx, EthereumBlockNumber, EthereumSlotNumber},
};

pub struct DepositEventExtractor {
    eth_api: EthApi,
    beacon_client: BeaconClient,

    erc20_treasury_address: H160,

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
    pub fn new(eth_api: EthApi, beacon_client: BeaconClient, erc20_treasury_address: H160) -> Self {
        Self {
            eth_api,
            beacon_client,

            erc20_treasury_address,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self, blocks: Receiver<EthereumBlockNumber>) -> Receiver<ERC20DepositTx> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Message queued extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<ERC20DepositTx>,
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
        sender: &Sender<ERC20DepositTx>,
    ) -> anyhow::Result<()> {
        let events = self
            .eth_api
            .fetch_deposit_events(self.erc20_treasury_address, block.0)
            .await?;

        let block_body = self
            .eth_api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(block.0), false)
            .await?
            .ok_or(anyhow!("Ethereum block #{} is missing", block.0))?;

        let beacon_root_parent = block_body.header.parent_beacon_block_root.ok_or(anyhow!(
            "Unable to determine root of parent beacon block for block #{}",
            block.0
        ))?;

        let beacon_block_parent = self
            .beacon_client
            .get_block_by_hash(&beacon_root_parent.0)
            .await?;

        let beacon_block = self
            .beacon_client
            .find_beacon_block(block.0, &beacon_block_parent)
            .await?;

        let slot_number = EthereumSlotNumber(beacon_block.slot);

        self.metrics
            .total_deposits_found
            .inc_by(events.len() as u64);

        for DepositEventEntry { tx_hash, .. } in events {
            sender.send(ERC20DepositTx {
                slot_number,
                tx_hash,
            })?;
        }

        Ok(())
    }
}
