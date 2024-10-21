use std::{
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

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
                log::error!("Deposit event extractor failed: {}", err);
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

        if events.is_empty() {
            return Ok(());
        }

        let slot_number = self.find_slot_by_block_number(block).await?;

        self.metrics
            .total_deposits_found
            .inc_by(events.len() as u64);

        for ev in &events {
            log::info!(
                "Found deposit event: tx_hash={}, from={}, to={}, token={}, amount={}, slot_number={}",
                hex::encode(ev.tx_hash.0),
                hex::encode(ev.from.0),
                hex::encode(ev.to.0),
                hex::encode(ev.token.0),
                ev.amount,
                slot_number.0,
            );
        }

        for DepositEventEntry { tx_hash, .. } in events {
            sender.send(ERC20DepositTx {
                slot_number,
                tx_hash,
            })?;
        }

        Ok(())
    }

    async fn find_slot_by_block_number(
        &self,
        block: EthereumBlockNumber,
    ) -> anyhow::Result<EthereumSlotNumber> {
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

        // TODO: It's a temporary solution of a problem that we're connecting to a different
        // nodes, so if we're observing finalized block on one node, the finalized slot might still be not
        // available on other.
        for _ in 0..10 {
            let beacon_block_result = self
                .beacon_client
                .find_beacon_block(block.0, &beacon_block_parent)
                .await;

            match beacon_block_result {
                Ok(beacon_block) => {
                    return Ok(EthereumSlotNumber(beacon_block.slot));
                }
                Err(err) => {
                    log::warn!(
                        "Failed to find beacon block for ethereum block #{}: {}. Waiting for 1 second before next attempt...",
                        block.0,
                        err
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        anyhow::bail!(
            "Failed to find beacon block for Ethereum block #{} after 5 attempts",
            block.0
        );
    }
}
