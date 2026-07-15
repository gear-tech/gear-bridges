use std::{
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use alloy::pubsub::Subscription;
use ethereum_client::{EthApi, PollingEthApi};
use futures::StreamExt;
use gear_common::api_provider::ApiProvider;
use relayer::{
    message_relayer::common::{
        gear::{
            block_listener::BlockListener,
            block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        },
        GearBlock,
    },
    rpc,
};

const DEFAULT_GEAR_ENDPOINT: &str = "wss://testnet-archive.vara.network";
const DEFAULT_GEAR_RPC_RETRIES: u8 = 3;
const DEFAULT_ETH_ENDPOINT: &str = "wss://hoodi-reth-rpc.gear-tech.io/ws";
const DEFAULT_ETH_MESSAGE_QUEUE_ADDRESS: &str = "0xAb8F315Cc80cf2368750fE5A33E259d6241b3dEB";

fn gear_endpoint() -> String {
    env::var("GEAR_ENDPOINT")
        .or_else(|_| env::var("GEAR_DOMAIN"))
        .unwrap_or_else(|_| DEFAULT_GEAR_ENDPOINT.to_string())
}

fn gear_retries() -> u8 {
    env::var("GEAR_RPC_RETRIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_GEAR_RPC_RETRIES)
}

fn eth_endpoint() -> String {
    env::var("ETHEREUM_RPC")
        .or_else(|_| env::var("ETHEREUM_ENDPOINT"))
        .or_else(|_| env::var("ETH_RPC"))
        .unwrap_or_else(|_| DEFAULT_ETH_ENDPOINT.to_string())
}

fn eth_message_queue_address() -> String {
    env::var("ETH_MESSAGE_QUEUE_ADDRESS")
        .unwrap_or_else(|_| DEFAULT_ETH_MESSAGE_QUEUE_ADDRESS.to_string())
}

async fn gear_latest_number(
    connection: &mut gear_common::api_provider::ApiProviderConnection,
) -> anyhow::Result<u32> {
    rpc::retry_gear(
        connection,
        "live test gear latest finalized",
        |api| async move {
            let hash = api.latest_finalized_block().await?;
            api.block_hash_to_number(hash).await
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_retry_helper_survives_explicit_reconnect() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut connection = provider.connection();
    provider.spawn();

    let before = gear_latest_number(&mut connection).await?;
    connection.reconnect().await?;
    let after = gear_latest_number(&mut connection).await?;

    assert!(
        after >= before,
        "latest finalized block regressed across reconnect: before={before}, after={after}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_provider_reconnects_multiple_connections() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut first = provider.connection();
    let mut second = provider.connection();
    provider.spawn();

    let baseline = gear_latest_number(&mut first).await?;
    first.reconnect().await?;
    let after_first_reconnect = gear_latest_number(&mut first).await?;
    second.reconnect().await?;
    let after_second_reconnect = gear_latest_number(&mut second).await?;

    assert!(
        after_first_reconnect >= baseline,
        "first connection regressed across reconnect: baseline={baseline}, after={after_first_reconnect}"
    );
    assert!(
        after_second_reconnect >= baseline,
        "second connection regressed after provider session refresh: baseline={baseline}, after={after_second_reconnect}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_worker_continues_while_sibling_triggers_reconnects() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut worker_connection = provider.connection();
    let mut reconnect_connection = provider.connection();
    provider.spawn();

    let worker = tokio::spawn(async move {
        let mut observed = Vec::new();
        for _ in 0..8 {
            let block = gear_latest_number(&mut worker_connection).await?;
            observed.push(block);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        anyhow::Ok(observed)
    });

    let reconnector = tokio::spawn(async move {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(125)).await;
            reconnect_connection.reconnect().await?;
            let _ = gear_latest_number(&mut reconnect_connection).await?;
        }
        anyhow::Ok(())
    });

    reconnector.await??;
    let observed = worker.await??;

    assert_eq!(
        observed.len(),
        8,
        "worker did not complete all RPC calls while sibling reconnected"
    );
    for pair in observed.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "worker observed finalized block regression while sibling reconnected: {observed:?}"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_subscription_can_be_recreated_after_reconnect() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut connection = provider.connection();
    provider.spawn();

    let subscription = connection
        .client()
        .subscribe_grandpa_justifications()
        .await?;
    drop(subscription);

    connection.reconnect().await?;

    let mut subscription = connection
        .client()
        .subscribe_grandpa_justifications()
        .await?;
    let justification = tokio::time::timeout(Duration::from_secs(45), subscription.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("GRANDPA justification stream ended after reconnect"))??;

    assert!(
        justification.commit.target_number > 0,
        "recreated GRANDPA subscription yielded an invalid block number"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_block_listener_background_catchup_still_emits_blocks() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut setup_connection = provider.connection();
    let listener_connection = provider.connection();
    provider.spawn();

    let latest = gear_latest_number(&mut setup_connection).await?;
    let from_block = latest.saturating_sub(8);
    let storage = Arc::new(CountingStorage {
        from_block,
        added: AtomicUsize::new(0),
    });

    let listener = BlockListener::new(listener_connection, storage.clone());
    let [mut blocks] = listener.run::<1>().await;

    let received = tokio::time::timeout(Duration::from_secs(90), blocks.recv()).await??;
    assert!(
        received.number() >= from_block,
        "received block {} before requested catch-up start {from_block}",
        received.number()
    );
    assert!(
        storage.added.load(Ordering::SeqCst) > 0,
        "listener emitted a block without recording it in replay storage"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Vara testnet RPC"]
async fn live_gear_background_catchup_replays_multiple_blocks() -> anyhow::Result<()> {
    let provider = ApiProvider::new(gear_endpoint(), gear_retries()).await?;
    let mut setup_connection = provider.connection();
    let listener_connection = provider.connection();
    provider.spawn();

    let latest = gear_latest_number(&mut setup_connection).await?;
    let from_block = latest.saturating_sub(12);
    let storage = Arc::new(CountingStorage {
        from_block,
        added: AtomicUsize::new(0),
    });

    let listener = BlockListener::new(listener_connection, storage.clone());
    let [_blocks] = listener.run::<1>().await;

    let added = storage.wait_for_added(4, Duration::from_secs(90)).await;
    assert!(
        added >= 4,
        "background catch-up did not replay enough blocks: added={added}, from_block={from_block}, latest={latest}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Hoodi Ethereum RPC"]
async fn live_ethereum_polling_reconnect_keeps_finalized_queries() -> anyhow::Result<()> {
    let api =
        PollingEthApi::new_with_retries(&eth_endpoint(), Some(3), Some(Duration::from_secs(2)))
            .await?;

    let before = api.finalized_block().await?.header.number;
    let api = api.reconnect().await?;
    let after = api.finalized_block().await?.header.number;
    let _block = api.get_block(after).await?;

    assert!(
        after >= before,
        "Ethereum finalized block regressed across reconnect: before={before}, after={after}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Hoodi Ethereum RPC"]
async fn live_ethereum_polling_survives_repeated_reconnects() -> anyhow::Result<()> {
    let mut api =
        PollingEthApi::new_with_retries(&eth_endpoint(), Some(3), Some(Duration::from_secs(2)))
            .await?;

    let baseline = api.finalized_block().await?.header.number;
    for attempt in 0..3 {
        api = api.reconnect().await?;
        let finalized = api.finalized_block().await?.header.number;
        let _block = api.get_block(finalized).await?;
        assert!(
            finalized >= baseline,
            "finalized block regressed on reconnect attempt {attempt}: baseline={baseline}, finalized={finalized}"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires live Hoodi Ethereum RPC"]
async fn live_ethereum_worker_queries_while_sibling_reconnects() -> anyhow::Result<()> {
    let worker_api =
        PollingEthApi::new_with_retries(&eth_endpoint(), Some(3), Some(Duration::from_secs(2)))
            .await?;
    let mut reconnect_api = worker_api.clone();

    let worker = tokio::spawn(async move {
        let mut observed = Vec::new();
        for _ in 0..8 {
            let finalized = worker_api.finalized_block().await?.header.number;
            let _ = worker_api.get_block(finalized).await?;
            observed.push(finalized);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        anyhow::Ok(observed)
    });

    let reconnector = tokio::spawn(async move {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(125)).await;
            reconnect_api = reconnect_api.reconnect().await?;
            let _ = reconnect_api.finalized_block().await?;
        }
        anyhow::Ok(())
    });

    reconnector.await??;
    let observed = worker.await??;

    assert_eq!(
        observed.len(),
        8,
        "Ethereum worker did not complete all RPC calls while sibling reconnected"
    );
    for pair in observed.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "Ethereum worker observed finalized block regression while sibling reconnected: {observed:?}"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Hoodi Ethereum RPC and MessageQueue contract"]
async fn live_ethereum_contract_reconnect_keeps_queries_and_subscriptions() -> anyhow::Result<()> {
    let api = EthApi::new_with_retries(
        &eth_endpoint(),
        &eth_message_queue_address(),
        None,
        Some(3),
        Some(Duration::from_secs(2)),
        None,
        None,
    )
    .await?;

    let max_block_number = api.max_block_number().await?;
    let finalized = api.finalized_block_number().await?;
    let from = finalized.saturating_sub(128);
    let _recent_roots = api.fetch_merkle_roots_in_range(from, finalized).await?;
    let before_subscription: Subscription<alloy::rpc::types::Log> = api.subscribe_logs().await?;
    drop(before_subscription);

    let api = api.reconnect().await?;
    let max_block_number_after = api.max_block_number().await?;
    let _max_block_distance = api.max_block_distance().await?;
    let after_subscription: Subscription<alloy::rpc::types::Log> = api.subscribe_logs().await?;
    drop(after_subscription);

    assert_eq!(
        max_block_number, max_block_number_after,
        "MessageQueue max block number changed across reconnect"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Hoodi Ethereum RPC and MessageQueue contract"]
async fn live_ethereum_block_subscription_recreated_after_reconnect() -> anyhow::Result<()> {
    let api = EthApi::new_with_retries(
        &eth_endpoint(),
        &eth_message_queue_address(),
        None,
        Some(3),
        Some(Duration::from_secs(2)),
        None,
        None,
    )
    .await?;

    let subscription = api.subscribe_blocks().await?;
    drop(subscription);

    let api = api.reconnect().await?;
    let subscription = api.subscribe_blocks().await?;
    let mut stream = subscription.into_result_stream();
    let header = tokio::time::timeout(Duration::from_secs(75), stream.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Ethereum block subscription ended after reconnect"))??;

    assert!(
        header.number > 0,
        "recreated Ethereum block subscription yielded an invalid block number"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires live Hoodi Ethereum RPC and MessageQueue contract"]
async fn live_ethereum_contract_range_queries_survive_repeated_reconnects() -> anyhow::Result<()> {
    let mut api = EthApi::new_with_retries(
        &eth_endpoint(),
        &eth_message_queue_address(),
        None,
        Some(3),
        Some(Duration::from_secs(2)),
        None,
        None,
    )
    .await?;

    let finalized = api.finalized_block_number().await?;
    let from = finalized.saturating_sub(256);
    let baseline_distance = api.max_block_distance().await?;

    for attempt in 0..3 {
        api = api.reconnect().await?;
        let distance = api.max_block_distance().await?;
        let _roots = api.fetch_merkle_roots_in_range(from, finalized).await?;
        assert_eq!(
            baseline_distance, distance,
            "MessageQueue max distance changed on reconnect attempt {attempt}"
        );
    }

    Ok(())
}

struct CountingStorage {
    from_block: u32,
    added: AtomicUsize,
}

impl CountingStorage {
    async fn wait_for_added(&self, expected: usize, timeout: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let added = self.added.load(Ordering::SeqCst);
            if added >= expected || tokio::time::Instant::now() >= deadline {
                return added;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[async_trait::async_trait]
impl UnprocessedBlocksStorage for CountingStorage {
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        UnprocessedBlocks {
            blocks: Vec::new(),
            first_block: Some((primitive_types::H256::zero(), self.from_block)),
            last_block: None,
        }
    }

    async fn add_block(
        &self,
        _api: &gear_rpc_client::GearApi,
        _block: &GearBlock,
    ) -> anyhow::Result<()> {
        self.added.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
