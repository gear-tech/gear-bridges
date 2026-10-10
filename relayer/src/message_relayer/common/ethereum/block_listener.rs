use crate::{
    common,
    message_relayer::common::{
        ethereum::block_storage::{finalized_block, BlockCursor, JSONBlockStorage},
        EthereumBlockNumber,
    },
};
use ethereum_client::PollingEthApi;
use prometheus::IntGauge;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);

pub struct BlockListener {
    eth_api: PollingEthApi,
    storage: Arc<JSONBlockStorage>,
    metrics: Metrics,
}

impl MeteredService for BlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_block: IntGauge = IntGauge::new(
            "ethereum_block_listener_latest_block",
            "Ethereum block listener latest block number",
        ),
    }
}

impl BlockListener {
    pub fn new(eth_api: PollingEthApi, storage: Arc<JSONBlockStorage>) -> Self {
        Self {
            eth_api,
            storage,
            metrics: Metrics::new(),
        }
    }

    pub fn spawn(self) -> Receiver<EthereumBlockNumber> {
        // One extracting, one queued, one durably recorded before a blocked send.
        let (sender, receiver) = channel(1);
        tokio::spawn(task(self, sender));
        receiver
    }

    async fn run_inner(&mut self, sender: &Sender<EthereumBlockNumber>) -> anyhow::Result<()> {
        let snapshot = self.storage.snapshot().await;
        for cursor in [snapshot.extracted, snapshot.discovered]
            .into_iter()
            .flatten()
        {
            let block = finalized_block(&self.eth_api, cursor.number).await?;
            anyhow::ensure!(
                block.header.hash.0 == cursor.hash.0,
                "HOLD: original finalized Ethereum discovery cursor changed"
            );
        }
        let mut next_block = snapshot.next_block()?;
        for (&number, &hash) in &snapshot.pending {
            let block = finalized_block(&self.eth_api, number).await?;
            anyhow::ensure!(
                block.header.hash.0 == hash.0,
                "HOLD: pending finalized Ethereum discovery hash changed"
            );
            sender.send(EthereumBlockNumber(number)).await?;
        }
        self.metrics.latest_block.set(next_block as i64);
        loop {
            let latest = self.eth_api.finalized_block().await?.header.number;
            if latest >= next_block {
                for number in next_block..=latest {
                    let block = finalized_block(&self.eth_api, number).await?;
                    persist_and_send_block(
                        self.storage.as_ref(),
                        sender,
                        BlockCursor {
                            number,
                            hash: block.header.hash.0.into(),
                        },
                        block.header.parent_hash.0.into(),
                    )
                    .await?;
                }
                next_block = latest
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("Ethereum block number overflow"))?;
                self.metrics.latest_block.set(latest as i64);
            } else {
                tokio::time::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2).await;
            }
        }
    }
}

async fn persist_and_send_block(
    storage: &JSONBlockStorage,
    sender: &Sender<EthereumBlockNumber>,
    block: BlockCursor,
    parent_hash: primitive_types::H256,
) -> anyhow::Result<()> {
    storage.add_block(block, parent_hash).await?;
    sender.send(EthereumBlockNumber(block.number)).await?;
    Ok(())
}

async fn task(mut this: BlockListener, sender: Sender<EthereumBlockNumber>) {
    loop {
        let Err(error) = this.run_inner(&sender).await else {
            continue;
        };
        log::error!("Ethereum block listener failed: {error:?}");
        if !common::is_transport_error_recoverable(&error) {
            log::error!("Non recoverable error, exiting.");
            return;
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            match this.eth_api.reconnect().await {
                Ok(api) => {
                    this.eth_api = api;
                    break;
                }
                Err(error) => {
                    log::error!("Failed to reconnect to Ethereum API: {error}. Retrying...");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::block_storage::tests::identity, *};
    use actix_web::{web, App, HttpResponse, HttpServer};
    use primitive_types::H256;
    use serde_json::{json, Value};
    use std::{net::TcpListener, sync::Mutex};

    #[tokio::test]
    async fn discovery_is_not_published_until_it_is_durable() {
        let path = std::env::temp_dir().join(format!("listener-{}.json", uuid::Uuid::now_v7()));
        let storage = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        let temporary = path.with_extension("tmp");
        tokio::fs::create_dir(&temporary).await.unwrap();
        let (sender, mut receiver) = channel(1);
        let block = BlockCursor {
            number: 70,
            hash: H256::repeat_byte(8),
        };
        assert!(
            persist_and_send_block(&storage, &sender, block, H256::repeat_byte(9))
                .await
                .is_err()
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(storage.snapshot().await.pending.is_empty());
        tokio::fs::remove_dir(&temporary).await.unwrap();
        persist_and_send_block(&storage, &sender, block, H256::repeat_byte(9))
            .await
            .unwrap();
        assert_eq!(receiver.recv().await.unwrap(), EthereumBlockNumber(70));
        let restored = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        assert_eq!(
            restored.snapshot().await.pending,
            std::collections::BTreeMap::from([(70, block.hash)])
        );
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn restarted_listener_replays_only_pending_and_new_finalized_blocks() {
        let mut blocks = Vec::new();
        let mut parent = alloy_primitives::B256::ZERO;
        for number in 0..=272 {
            let mut block: alloy::rpc::types::Block = Default::default();
            block.header.inner.number = number;
            block.header.inner.parent_hash = parent;
            block.header.inner.timestamp = number * 12;
            if number == 0 {
                block.header.inner.extra_data =
                    alloy_primitives::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes());
            }
            block.header.hash = block.header.inner.hash_slow();
            parent = block.header.hash;
            blocks.push(block);
        }
        let blocks = Arc::new(blocks);
        let queries = Arc::new(Mutex::new(Vec::new()));
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", socket.local_addr().unwrap());
        let server_blocks = blocks.clone();
        let server_queries = queries.clone();
        let server = HttpServer::new(move || {
            let blocks = server_blocks.clone();
            let queries = server_queries.clone();
            App::new().route(
                "/",
                web::post().to(move |request: web::Json<Value>| {
                    let blocks = blocks.clone();
                    let queries = queries.clone();
                    async move {
                        let result = match request["method"].as_str().unwrap() {
                            "eth_chainId" => json!("0x88bb0"),
                            "eth_getBlockByNumber" => {
                                let tag = request["params"][0].as_str().unwrap();
                                let number = if tag == "finalized" || tag == "latest" {
                                    272
                                } else {
                                    u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
                                };
                                queries.lock().unwrap().push(number);
                                serde_json::to_value(&blocks[number as usize]).unwrap()
                            }
                            "eth_getBlockByHash" => {
                                let hash = request["params"][0].as_str().unwrap();
                                let block = blocks
                                    .iter()
                                    .find(|block| format!("{:#x}", block.header.hash) == hash)
                                    .unwrap();
                                queries.lock().unwrap().push(block.header.number);
                                serde_json::to_value(block).unwrap()
                            }
                            method => panic!("unexpected read-only RPC method {method}"),
                        };
                        HttpResponse::Ok().json(
                            json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }),
                        )
                    }
                }),
            )
        })
        .workers(1)
        .listen(socket)
        .unwrap()
        .disable_signals()
        .run();
        let server_handle = server.handle();
        tokio::spawn(server);
        let api = PollingEthApi::new(&endpoint).await.unwrap();
        assert!(api
            .is_finalized_block(0, blocks[0].header.hash.0.into())
            .await
            .unwrap());
        queries.lock().unwrap().clear();
        let path =
            std::env::temp_dir().join(format!("listener-aged-{}.json", uuid::Uuid::now_v7()));
        let mut bound = identity();
        bound.ethereum_genesis_hash = blocks[0].header.hash.0.into();
        let storage = Arc::new(
            JSONBlockStorage::new(path.clone(), bound.clone())
                .await
                .unwrap(),
        );
        for number in 70..=270 {
            let block = &blocks[number as usize];
            let cursor = BlockCursor {
                number,
                hash: block.header.hash.0.into(),
            };
            storage
                .add_block(cursor, block.header.parent_hash.0.into())
                .await
                .unwrap();
            if number < 270 {
                storage.acknowledge_block(cursor).await.unwrap();
            }
        }
        drop(storage);
        let restored = Arc::new(JSONBlockStorage::new(path.clone(), bound).await.unwrap());
        let (sender, mut receiver) = channel(1);
        let mut listener = BlockListener::new(api, restored.clone());
        let actor = tokio::spawn(async move { listener.run_inner(&sender).await });
        for expected in 270..=272 {
            let actual = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(actual, EthereumBlockNumber(expected));
            restored
                .acknowledge_block(BlockCursor {
                    number: expected,
                    hash: blocks[expected as usize].header.hash.0.into(),
                })
                .await
                .unwrap();
        }
        actor.abort();
        assert!(actor.await.unwrap_err().is_cancelled());
        assert!(queries
            .lock()
            .unwrap()
            .iter()
            .all(|&number| number == 0 || number >= 269));
        assert_eq!(restored.snapshot().await.next_block().unwrap(), 273);
        assert!(restored.snapshot().await.pending.is_empty());
        server_handle.stop(true).await;
        tokio::fs::remove_file(path).await.unwrap();
    }
}
