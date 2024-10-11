use std::sync::mpsc::{channel, Receiver, Sender};

use ethereum_client::EthApi;
use futures::executor::block_on;
use prometheus::IntCounter;
use sails_rs::H160;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::common::{ERC20DepositTx, EthereumBlockNumber};

pub struct DepositEventExtractor {
    eth_api: EthApi,
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
    pub fn new(eth_api: EthApi, erc20_treasury_address: H160) -> Self {
        Self {
            eth_api,
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
        _sender: &Sender<ERC20DepositTx>,
    ) -> anyhow::Result<()> {
        let _events = self
            .eth_api
            .fetch_deposit_events(self.erc20_treasury_address, block.0)
            .await?;

        // // TODO: fetch slot number by block number.
        // let slot_number = todo!();

        // self.metrics
        //     .total_deposits_found
        //     .inc_by(events.len() as u64);

        // for DepositEventEntry { tx_hash, .. } in events {
        //     sender.send(ERC20DepositTx {
        //         slot_number,
        //         tx_hash,
        //     })?;
        // }

        Ok(())
    }
}
