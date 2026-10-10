use ethereum_client::{DepositEventEntry, PollingEthApi};
use primitive_types::H160;
use prometheus::IntCounter;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, Receiver, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{self, BASE_RETRY_DELAY},
    message_relayer::{
        common::{
            ethereum::{
                block_listener::ETHEREUM_BLOCK_TIME_APPROX,
                block_storage::{
                    finalized_block, replay_pending_handoffs, BlockCursor, JSONBlockStorage,
                },
            },
            EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot,
        },
        eth_to_gear::storage::Storage,
    },
};

pub struct DepositEventExtractor {
    eth_api: PollingEthApi,

    erc20_manager_address: H160,

    storage: Arc<dyn Storage>,
    discovery: Arc<JSONBlockStorage>,

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
        discovery: Arc<JSONBlockStorage>,
    ) -> Self {
        Self {
            eth_api,

            erc20_manager_address,
            storage,
            discovery,
            genesis_time,

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut blocks: Receiver<EthereumBlockNumber>,
    ) -> UnboundedReceiver<TxHashWithSlot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(async move {
            let mut attempts: u32 = 0;
            let mut unprocessed = Vec::new();

            loop {
                let res = self.run_inner(&sender, &mut blocks, &mut unprocessed).await;
                if let Err(err) = res {
                    attempts += 1;
                    if !common::is_transport_error_recoverable(&err) {
                        log::error!(
                            "Non recoverable deposit event extractor error, exiting: {err:#}"
                        );
                        return;
                    }
                    let delay = BASE_RETRY_DELAY * 2u32.pow(attempts.saturating_sub(1).min(6));

                    log::error!(
                        "Deposit event extractor failed with recoverable error (attempt {attempts}): {err}. Retrying in {delay:?}"
                    );
                    tokio::time::sleep(delay).await;
                    loop {
                        match self.eth_api.reconnect().await {
                            Ok(api) => {
                                self.eth_api = api;
                                break;
                            }
                            Err(err) => {
                                log::error!("Failed to reconnect to Ethereum: {err}. Retrying...");
                                tokio::time::sleep(BASE_RETRY_DELAY).await;
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
        let events = self
            .eth_api
            .fetch_deposit_events_at(self.erc20_manager_address, block.0, cursor.hash)
            .await?;
        let timestamp = header
            .header
            .timestamp
            .checked_sub(self.genesis_time)
            .ok_or_else(|| {
                anyhow::anyhow!("HOLD: Ethereum block timestamp precedes Beacon genesis")
            })?;
        let slot_number = EthereumSlotNumber(timestamp / ETHEREUM_BLOCK_TIME_APPROX.as_secs());
        super::block_storage::store_extracted_transactions(
            self.storage.as_ref(),
            &self.discovery,
            slot_number,
            cursor,
            events.iter().map(|event| event.tx_hash),
        )
        .await?;
        let mut total = 0;
        for DepositEventEntry {
            tx_hash,
            from,
            to,
            token,
            amount,
        } in events
        {
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
                "Found deposit event: tx_hash={}, from={}, to={}, token={}, amount={}, slot_number={}",
                hex::encode(tx_hash.0), hex::encode(from.0), hex::encode(to.0),
                hex::encode(token.0), amount, slot_number.0,
            );
            sender.send(TxHashWithSlot {
                slot_number,
                tx_hash,
            })?;
        }
        self.metrics.total_deposits_found.inc_by(total);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::eth_to_gear::storage::NoStorage;
    use actix_web::{web, App, HttpResponse, HttpServer};
    use std::{net::TcpListener, time::Duration};

    #[tokio::test]
    async fn transport_failure_retains_replayed_and_live_blocks() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = HttpServer::new(|| {
            App::new().route(
                "/",
                web::post().to(|| async { HttpResponse::ServiceUnavailable().finish() }),
            )
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .disable_signals()
        .run();
        let handle = server.handle();
        tokio::spawn(server);
        let path =
            std::env::temp_dir().join(format!("deposit-discovery-{}.json", uuid::Uuid::now_v7()));
        let discovery = Arc::new(
            JSONBlockStorage::new(path.clone(), super::super::block_storage::tests::identity())
                .await
                .unwrap(),
        );
        let extractor = DepositEventExtractor::new(
            PollingEthApi::new(&endpoint).await.unwrap(),
            H160::zero(),
            Arc::new(NoStorage::new()),
            0,
            discovery,
        );
        let mut retained = Vec::new();
        for replay in [true, false] {
            let (blocks_tx, mut blocks) = tokio::sync::mpsc::channel(1);
            let (sender, _receiver) = unbounded_channel();
            let mut pending = if replay {
                vec![EthereumBlockNumber(7)]
            } else {
                Vec::new()
            };
            if !replay {
                blocks_tx.send(EthereumBlockNumber(7)).await.unwrap();
            }
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                extractor.run_inner(&sender, &mut blocks, &mut pending),
            )
            .await
            .unwrap();
            assert!(result.is_err());
            retained.push(pending.iter().map(|block| block.0).collect::<Vec<_>>());
        }
        handle.stop(true).await;
        assert_eq!(retained, vec![vec![7], vec![7]]);
        tokio::fs::remove_file(path).await.unwrap();
    }
}
