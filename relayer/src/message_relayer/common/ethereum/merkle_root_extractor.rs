use crate::message_relayer::common::{AuthoritySetId, GearBlockNumber, RelayedMerkleRoot};
use alloy::{
    providers::{PendingTransactionBuilder, Provider},
    sol_types::SolEvent,
};
use ethereum_client::{abi::IMessageQueue::MerkleRoot, EthApi};
use futures::StreamExt;
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::GearApi;
use primitive_types::H256;
use prometheus::IntGauge;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use utils_prometheus::{impl_metered_service, MeteredService};

const ETH_RPC_HEALTHCHECK_INTERVAL: Duration = Duration::from_secs(15);
const ETH_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const ETH_CATCH_UP_LOOKBACK_BLOCKS: u64 = 2_000;

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    confirmations: u64,
    sender: UnboundedSender<RelayedMerkleRoot>,
    last_scanned_eth_block: Option<u64>,

    metrics: Metrics,
}

impl MeteredService for MerkleRootExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_merkle_root_for_block: IntGauge = IntGauge::new(
            "merkle_root_extractor_latest_merkle_root_for_block",
            "Latest gear block present in found merkle roots",
        ),
        latest_ethereum_block: IntGauge = IntGauge::new(
            "merkle_root_extractor_latest_ethereum_block",
            "Latest Ethereum block observed by the merkle root extractor healthcheck",
        ),
    }
}

impl MerkleRootExtractor {
    pub fn new(
        eth_api: EthApi,
        api_provider: ApiProviderConnection,
        confirmations: u64,
        sender: UnboundedSender<RelayedMerkleRoot>,
    ) -> Self {
        Self {
            eth_api,
            api_provider,
            confirmations,
            sender,
            last_scanned_eth_block: None,

            metrics: Metrics::new(),
        }
    }

    pub fn sender(&self) -> &UnboundedSender<RelayedMerkleRoot> {
        &self.sender
    }

    pub fn spawn(self) {
        tokio::task::spawn(task(self));
    }

    async fn fetch_hash_auth_id(
        &mut self,
        block_number_gear: u32,
    ) -> Option<(H256, AuthoritySetId)> {
        loop {
            let gear_api = self.api_provider.client();

            match self::fetch_hash_auth_id(&gear_api, block_number_gear).await {
                Ok(result) => return Some(result),

                Err(e) => {
                    log::error!(r#"Merkle root extractor failed to fetch block_hash: "{e:?}""#);
                    log::trace!(
                        r#"e.downcast_ref::<gsdk::Error>(): "{:?}""#,
                        e.downcast_ref::<gsdk::Error>()
                    );
                    log::trace!(
                        r#"e.downcast_ref::<subxt::Error>(): "{:?}""#,
                        e.downcast_ref::<subxt::Error>()
                    );
                    for cause in e.chain() {
                        log::trace!(r#"cause: "{cause:?}""#);
                    }
                }
            }

            if let Err(e) = self.api_provider.reconnect().await {
                log::error!(r#"Merkle root extractor unable to reconnect: "{e}""#);
                return None;
            }

            log::debug!("API provider reconnected");
        }
    }

    async fn send_root(
        &mut self,
        block_number_gear: u32,
        merkle_root: H256,
        block_number_eth: u64,
    ) -> anyhow::Result<bool> {
        let block_timestamp = self.eth_api.get_block_timestamp(block_number_eth).await?;

        log::info!(
            "Found merkle root {:?} at Ethereum block #{block_number_eth} with timestamp {block_timestamp}",
            (block_number_gear, merkle_root),
        );

        self.metrics
            .latest_merkle_root_for_block
            .set(block_number_gear as i64);

        let Some((block_hash, authority_set_id)) = self.fetch_hash_auth_id(block_number_gear).await
        else {
            return Ok(false);
        };

        log::info!(
            "Merkle root {:?} is for era #{authority_set_id}",
            (block_number_gear, merkle_root),
        );

        if let Err(e) = self.sender.send(RelayedMerkleRoot {
            block: GearBlockNumber(block_number_gear),
            block_hash,
            authority_set_id,
            merkle_root,
            timestamp: block_timestamp,
        }) {
            log::error!(r#"Sender channel closed: "{e:?}"."#);
            return Ok(false);
        }

        Ok(true)
    }

    async fn check_eth_rpc_and_catch_up(&mut self) -> anyhow::Result<bool> {
        let latest_block =
            tokio::time::timeout(ETH_RPC_REQUEST_TIMEOUT, self.eth_api.latest_block_number())
                .await
                .map_err(|_| anyhow::anyhow!("Ethereum RPC healthcheck timed out"))??;

        self.metrics.latest_ethereum_block.set(latest_block as i64);

        // Only poll blocks with the same confirmation margin as the live event path.
        let confirmed_block = latest_block.saturating_sub(self.confirmations);
        let Some((from, to)) = catch_up_range(
            self.last_scanned_eth_block,
            confirmed_block,
            ETH_CATCH_UP_LOOKBACK_BLOCKS,
        ) else {
            return Ok(true);
        };

        let roots = tokio::time::timeout(
            ETH_RPC_REQUEST_TIMEOUT,
            self.eth_api.fetch_merkle_roots_in_range(from, to),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Ethereum merkle root catch-up timed out"))??;

        if !roots.is_empty() {
            log::info!(
                "Merkle root extractor catch-up found {} root(s) in Ethereum blocks #{from}..=#{to}",
                roots.len(),
            );
        }

        for (root, block_number_eth) in roots {
            let Some(block_number_eth) = block_number_eth else {
                log::warn!("Merkle root catch-up returned an event without an Ethereum block");
                continue;
            };

            if !self
                .send_root(root.block_number as u32, root.merkle_root, block_number_eth)
                .await?
            {
                return Ok(false);
            }
        }

        // Advance only after the complete range was fetched and forwarded. A failure retries it.
        self.last_scanned_eth_block = Some(to);
        Ok(true)
    }
}

fn catch_up_range(
    last_scanned: Option<u64>,
    confirmed_block: u64,
    initial_lookback: u64,
) -> Option<(u64, u64)> {
    let from = match last_scanned {
        Some(last_scanned) => last_scanned.saturating_add(1),
        None => confirmed_block.saturating_sub(initial_lookback.saturating_sub(1)),
    };
    (from <= confirmed_block).then_some((from, confirmed_block))
}

async fn fetch_hash_auth_id(
    gear_api: &GearApi,
    block_number_gear: u32,
) -> anyhow::Result<(H256, AuthoritySetId)> {
    let block_hash = gear_api.block_number_to_hash(block_number_gear).await?;

    let authority_set_id = AuthoritySetId(gear_api.signed_by_authority_set_id(block_hash).await?);

    Ok((block_hash, authority_set_id))
}

async fn task(mut this: MerkleRootExtractor) {
    loop {
        let Err(err) = task_inner(&mut this).await else {
            log::info!("Exiting");
            break;
        };

        log::error!(r#"Merkle root extractor failed: "{err:?}""#);

        loop {
            match this.eth_api.reconnect().await {
                Ok(eth_api) => {
                    this.eth_api = eth_api;
                    break;
                }
                Err(err) => {
                    log::error!(r#"Failed to reconnect to Ethereum: "{err}". Retrying..."#);
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
    }
}

async fn task_inner(this: &mut MerkleRootExtractor) -> anyhow::Result<()> {
    let subscription = this.eth_api.subscribe_logs().await?;

    let mut stream = subscription.into_result_stream();
    // A log subscription can silently stop yielding without returning an error. Probe the
    // Ethereum RPC and poll confirmed gaps so reconnects cannot lose MerkleRoot events.
    let mut interval = tokio::time::interval(ETH_RPC_HEALTHCHECK_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !this.api_provider.is_alive() {
                    return Err(anyhow::anyhow!("ApiProvider connection is dead"));
                }

                if !this.check_eth_rpc_and_catch_up().await? {
                    return Ok(());
                }
            }

            log = stream.next() => {
                let log = match log {
                    Some(Ok(log)) => log,
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("Failed to get first log from stream: {e:?}"));
                    }
                    None => {
                        return Err(anyhow::anyhow!("Log stream closed"));
                    }
                };

                log::debug!("Get log = {log:?}");

                let (Some(tx_hash), Some(block_number)) = (log.transaction_hash, log.block_number) else {
                    log::error!("Unable to get tx_hash and block_number for log = {log:?}. Skipping");
                    continue;
                };

                if log.removed {
                    log::debug!("Blocks reorganization, log = {log:?}. Skipping");
                    continue;
                }

                let root = match MerkleRoot::decode_log_data(log.data()) {
                    Ok(root) => root,
                    Err(e) => {
                        log::error!("Failed to decode log = {log:?}: {e:?}. Skipping");
                        continue;
                    }
                };

                let pending =
                    PendingTransactionBuilder::new(this.eth_api.raw_provider().root().clone(), tx_hash);
                pending
                    .with_required_confirmations(this.confirmations)
                    .watch()
                    .await?;

                let block_number_gear: u32 = root.blockNumber.to();
                log::debug!(
                    "Merkle root {:?} reached {} confirmation(s)",
                    (root.blockNumber, root.merkleRoot),
                    this.confirmations,
                );

                if !this.send_root(
                    block_number_gear,
                    root.merkleRoot.0.into(),
                    block_number,
                ).await? {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::catch_up_range;

    #[test]
    fn catch_up_starts_after_last_scanned_block() {
        assert_eq!(catch_up_range(Some(100), 105, 2_000), Some((101, 105)));
    }

    #[test]
    fn catch_up_waits_until_a_new_block_is_confirmed() {
        assert_eq!(catch_up_range(Some(105), 105, 2_000), None);
        assert_eq!(catch_up_range(Some(106), 105, 2_000), None);
    }

    #[test]
    fn first_catch_up_scans_recent_confirmed_blocks() {
        assert_eq!(catch_up_range(None, 5_000, 2_000), Some((3_001, 5_000)));
        assert_eq!(catch_up_range(None, 100, 2_000), Some((0, 100)));
    }
}
