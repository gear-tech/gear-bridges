use crate::{
    message_relayer::common::{AuthoritySetId, GearBlockNumber, RelayedMerkleRoot},
    rpc,
};
use anyhow::Context;
use ethereum_client::{EthApi, MerkleRootEntry};
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::GearApi;
use primitive_types::H256;
use prometheus::{IntCounter, IntGauge};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use utils_prometheus::{impl_metered_service, MeteredService};

const POLL_INTERVAL: Duration = Duration::from_secs(15);
const RECONNECT_DELAY: Duration = Duration::from_secs(30);
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const QUERY_CHUNK_SIZE: u64 = 2_000;
const STARTUP_HISTORY_BLOCKS: u64 = 100_000;
const REORG_LOOKBACK_BLOCKS: u64 = 64;

#[derive(Debug, Error)]
#[error("Gear operation failed: {source}")]
struct GearOperationError {
    #[source]
    source: anyhow::Error,
}

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    confirmations: u64,
    sender: UnboundedSender<RelayedMerkleRoot>,

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
        last_scanned_eth_block: IntGauge = IntGauge::new(
            "merkle_root_extractor_last_scanned_eth_block",
            "Latest Ethereum block successfully scanned for merkle roots",
        ),
        scan_lag_blocks: IntGauge = IntGauge::new(
            "merkle_root_extractor_scan_lag_blocks",
            "Confirmed Ethereum blocks not yet scanned for merkle roots",
        ),
        last_success_timestamp: IntGauge = IntGauge::new(
            "merkle_root_extractor_last_success_timestamp",
            "Unix timestamp of the latest successful merkle root reconciliation",
        ),
        reconnects_total: IntCounter = IntCounter::new(
            "merkle_root_extractor_reconnects_total",
            "Total successful Ethereum API reconnects by the merkle root extractor",
        ),
        scan_failures_total: IntCounter = IntCounter::new(
            "merkle_root_extractor_scan_failures_total",
            "Total failed merkle root reconciliation attempts",
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
    ) -> anyhow::Result<(H256, AuthoritySetId)> {
        rpc::retry_gear(
            &mut self.api_provider,
            "fetch merkle root Gear block metadata",
            move |gear_api| async move {
                self::fetch_hash_auth_id(&gear_api, block_number_gear).await
            },
        )
        .await
        .map_err(|source| GearOperationError { source }.into())
    }

    async fn scan_range(&mut self, range: ScanRange) -> anyhow::Result<()> {
        let roots = tokio::time::timeout(
            RPC_TIMEOUT,
            self.eth_api
                .fetch_merkle_roots_in_range(range.from, range.to),
        )
        .await
        .with_context(|| {
            format!(
                "Timed out fetching merkle roots from Ethereum blocks #{}..=#{}",
                range.from, range.to
            )
        })??;

        let roots_len = roots.len();
        for (root, eth_block) in roots {
            let eth_block = eth_block.with_context(|| {
                format!(
                    "Merkle root for Gear block #{} has no Ethereum block number",
                    root.block_number
                )
            })?;
            self.process_root(root, eth_block).await?;
        }

        log::trace!(
            "Successfully scanned Ethereum blocks #{}..=#{} and found {roots_len} merkle root entry(ies)",
            range.from,
            range.to,
        );
        Ok(())
    }

    async fn process_root(&mut self, root: MerkleRootEntry, eth_block: u64) -> anyhow::Result<()> {
        let block_timestamp =
            tokio::time::timeout(RPC_TIMEOUT, self.eth_api.get_block_timestamp(eth_block))
                .await
                .with_context(|| {
                    format!("Timed out fetching timestamp for Ethereum block #{eth_block}")
                })??;

        let block_number_gear = u32::try_from(root.block_number).with_context(|| {
            format!(
                "Merkle root Gear block number does not fit into u32: {}",
                root.block_number
            )
        })?;

        let latest_root_block = self.metrics.latest_merkle_root_for_block.get();
        self.metrics
            .latest_merkle_root_for_block
            .set(latest_root_block.max(i64::from(block_number_gear)));

        let (block_hash, authority_set_id) = self.fetch_hash_auth_id(block_number_gear).await?;

        log::info!(
            "Found merkle root {:?} at Ethereum block #{eth_block} with timestamp {block_timestamp} ({} confirmation(s)), for era #{authority_set_id}",
            (root.block_number, root.merkle_root),
            self.confirmations,
        );

        self.sender
            .send(RelayedMerkleRoot {
                block: GearBlockNumber(block_number_gear),
                block_hash,
                authority_set_id,
                merkle_root: root.merkle_root,
                timestamp: block_timestamp,
            })
            .context("Merkle root receiver channel closed")
    }
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
    let mut scan_state = ScanState::default();
    // Keep the interval across failures so a retained range cannot be replayed in a tight loop.
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        let Err(err) = task_inner(&mut this, &mut scan_state).await else {
            continue;
        };

        this.metrics.scan_failures_total.inc();
        log::error!(r#"Merkle root extractor failed: "{err:?}""#);

        if this.sender.is_closed() {
            log::info!("Merkle root receiver channel closed, exiting");
            break;
        }

        if !should_reconnect_ethereum(&err) {
            continue;
        }

        loop {
            match this.eth_api.reconnect().await {
                Ok(eth_api) => {
                    this.eth_api = eth_api;
                    this.metrics.reconnects_total.inc();
                    break;
                }
                Err(err) => {
                    log::error!(r#"Failed to reconnect to Ethereum: "{err}". Retrying..."#);
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            }
        }
    }
}

async fn task_inner(
    this: &mut MerkleRootExtractor,
    scan_state: &mut ScanState,
) -> anyhow::Result<()> {
    if !this.api_provider.is_alive() {
        this.api_provider
            .reconnect()
            .await
            .map_err(|source| GearOperationError { source })?;
    }

    scan_once(this, scan_state).await
}

fn should_reconnect_ethereum(err: &anyhow::Error) -> bool {
    err.downcast_ref::<GearOperationError>().is_none()
        && rpc::classify_anyhow(err) == rpc::RetryDecision::Retry
}

async fn scan_once(
    this: &mut MerkleRootExtractor,
    scan_state: &mut ScanState,
) -> anyhow::Result<()> {
    let latest = tokio::time::timeout(RPC_TIMEOUT, this.eth_api.latest_block_number())
        .await
        .context("Timed out fetching latest Ethereum block number")??;
    let confirmed = confirmed_head(latest, this.confirmations);

    scan_state.initialize(confirmed);
    update_scan_lag(&this.metrics, scan_state, confirmed);

    while let Some(range) = scan_state.next_forward(confirmed) {
        let result = this.scan_range(range).await;
        scan_state.finish(range, result)?;

        this.metrics
            .last_scanned_eth_block
            .set(u64_to_i64(range.to));
        update_scan_lag(&this.metrics, scan_state, confirmed);
    }

    // Re-scan a short overlap because `confirmed` need not be finalized and a reorg may
    // replace an Ethereum block after its first successful scan.
    let range = scan_state.reconciliation_range(confirmed);
    this.scan_range(range).await?;

    // Backfill one historical chunk per poll after current confirmed blocks are caught up.
    if let Some(range) = scan_state.next_backfill() {
        let result = this.scan_range(range).await;
        scan_state.finish(range, result)?;
    }

    this.metrics
        .last_success_timestamp
        .set(u64_to_i64(unix_timestamp()));

    Ok(())
}

fn confirmed_head(latest: u64, confirmations: u64) -> u64 {
    latest.saturating_sub(confirmations.saturating_sub(1))
}

fn update_scan_lag(metrics: &Metrics, scan_state: &ScanState, confirmed: u64) {
    let last_scanned = scan_state
        .forward_next
        .and_then(|next| next.checked_sub(1))
        .unwrap_or_default();
    metrics
        .scan_lag_blocks
        .set(u64_to_i64(confirmed.saturating_sub(last_scanned)));
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanKind {
    Forward,
    Backfill,
    Reconciliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScanRange {
    kind: ScanKind,
    from: u64,
    to: u64,
}

#[derive(Debug, Default)]
struct ScanState {
    initialized: bool,
    forward_next: Option<u64>,
    backfill_floor: u64,
    backfill_next: Option<u64>,
}

impl ScanState {
    fn initialize(&mut self, confirmed: u64) {
        if self.initialized {
            return;
        }

        let forward_from = confirmed.saturating_sub(QUERY_CHUNK_SIZE - 1);
        self.forward_next = Some(forward_from);
        self.backfill_floor = confirmed.saturating_sub(STARTUP_HISTORY_BLOCKS - 1);
        self.backfill_next = forward_from
            .checked_sub(1)
            .filter(|to| *to >= self.backfill_floor);
        self.initialized = true;
    }

    fn next_forward(&self, confirmed: u64) -> Option<ScanRange> {
        let from = self.forward_next?;
        if from > confirmed {
            return None;
        }

        Some(ScanRange {
            kind: ScanKind::Forward,
            from,
            to: confirmed.min(from.saturating_add(QUERY_CHUNK_SIZE - 1)),
        })
    }

    fn reconciliation_range(&self, confirmed: u64) -> ScanRange {
        ScanRange {
            kind: ScanKind::Reconciliation,
            from: confirmed.saturating_sub(REORG_LOOKBACK_BLOCKS - 1),
            to: confirmed,
        }
    }

    fn next_backfill(&self) -> Option<ScanRange> {
        let to = self.backfill_next?;
        Some(ScanRange {
            kind: ScanKind::Backfill,
            from: self
                .backfill_floor
                .max(to.saturating_sub(QUERY_CHUNK_SIZE - 1)),
            to,
        })
    }

    fn finish(&mut self, range: ScanRange, result: anyhow::Result<()>) -> anyhow::Result<()> {
        result?;

        match range.kind {
            ScanKind::Forward => {
                debug_assert_eq!(self.forward_next, Some(range.from));
                self.forward_next = range.to.checked_add(1);
            }
            ScanKind::Backfill => {
                debug_assert_eq!(self.backfill_next, Some(range.to));
                self.backfill_next = if range.from <= self.backfill_floor {
                    None
                } else {
                    range.from.checked_sub(1)
                };
            }
            ScanKind::Reconciliation => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_failures_do_not_reconnect_ethereum() {
        let err = anyhow::Error::new(GearOperationError {
            source: anyhow::anyhow!("backend connection task has stopped"),
        });

        assert!(!should_reconnect_ethereum(&err));
        assert!(should_reconnect_ethereum(&anyhow::anyhow!(
            "backend connection task has stopped"
        )));
    }

    #[test]
    fn confirmation_horizon_includes_the_event_block_as_first_confirmation() {
        assert_eq!(confirmed_head(100, 0), 100);
        assert_eq!(confirmed_head(100, 1), 100);
        assert_eq!(confirmed_head(100, 2), 99);
        assert_eq!(confirmed_head(100, 12), 89);
    }

    #[test]
    fn scans_recent_blocks_before_backfilling_history() {
        let mut state = ScanState::default();
        state.initialize(10_000);

        let recent = state.next_forward(10_000).unwrap();
        assert_eq!((recent.from, recent.to), (8_001, 10_000));
        state.finish(recent, Ok(())).unwrap();
        assert!(state.next_forward(10_000).is_none());

        let backfill = state.next_backfill().unwrap();
        assert_eq!((backfill.from, backfill.to), (6_001, 8_000));

        let reconciliation = state.reconciliation_range(10_000);
        assert_eq!((reconciliation.from, reconciliation.to), (9_937, 10_000));
    }

    #[test]
    fn failed_forward_scan_does_not_advance_cursor() {
        let mut state = ScanState::default();
        state.initialize(10_000);
        let range = state.next_forward(10_000).unwrap();

        assert!(state
            .finish(range, Err(anyhow::anyhow!("provider unavailable")))
            .is_err());
        assert_eq!(state.next_forward(10_000), Some(range));
    }

    #[test]
    fn failed_backfill_scan_does_not_advance_cursor() {
        let mut state = ScanState::default();
        state.initialize(10_000);
        let recent = state.next_forward(10_000).unwrap();
        state.finish(recent, Ok(())).unwrap();
        let range = state.next_backfill().unwrap();

        assert!(state
            .finish(range, Err(anyhow::anyhow!("provider unavailable")))
            .is_err());
        assert_eq!(state.next_backfill(), Some(range));
    }

    #[test]
    fn forward_scan_resumes_from_first_unscanned_block() {
        let mut state = ScanState::default();
        state.initialize(10_000);
        let initial = state.next_forward(10_000).unwrap();
        state.finish(initial, Ok(())).unwrap();

        let resumed = state.next_forward(12_500).unwrap();
        assert_eq!((resumed.from, resumed.to), (10_001, 12_000));
        state.finish(resumed, Ok(())).unwrap();

        let tail = state.next_forward(12_500).unwrap();
        assert_eq!((tail.from, tail.to), (12_001, 12_500));
    }
}
