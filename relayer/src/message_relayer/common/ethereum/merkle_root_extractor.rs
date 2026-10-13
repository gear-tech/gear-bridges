use super::accumulator::{
    utils::{MerkleRoots, RootAddError},
    RootUpdate,
};
use crate::message_relayer::common::{
    AuthoritySetId, EthereumRootInclusion, GearBlockNumber, QualifiedMerkleRoot, RelayedMerkleRoot,
};
use alloy::{eips::BlockNumberOrTag, primitives::B256, providers::Provider};
use anyhow::{Context, Result};
use ethereum_client::{EthApi, FinalizedView, MerkleRootObservation, QueueDeployment};
use futures::StreamExt;
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::GearApi;
use primitive_types::H256;
use prometheus::IntGauge;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::mpsc::UnboundedSender;
use utils_prometheus::{impl_metered_service, MeteredService};

const ETH_RPC_HEALTHCHECK_INTERVAL: Duration = Duration::from_secs(15);
const ETH_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const ETH_SCAN_BATCH_BLOCKS: u64 = 2_000;
const ROOT_CAPACITY: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EthereumRootCursor {
    pub block_number: u64,
    pub block_hash: B256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EthereumRootPhase {
    Rebuilding,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EthereumRootState {
    pub schema_version: u32,
    pub phase: EthereumRootPhase,
    pub scan_start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_deployment: Option<QueueDeployment>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub migration_target: Option<EthereumRootCursor>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub cursor: Option<EthereumRootCursor>,
    pub roots: Vec<QualifiedMerkleRoot>,
}

impl EthereumRootState {
    pub fn rebuilding(target: EthereumRootCursor) -> Self {
        Self {
            schema_version: 1,
            phase: EthereumRootPhase::Rebuilding,
            scan_start: 0,
            queue_deployment: None,
            migration_target: Some(target),
            cursor: None,
            roots: Vec::new(),
        }
    }

    fn bind_deployment(&mut self, deployment: QueueDeployment) -> Result<()> {
        deployment.validate()?;
        if let Some(saved) = &self.queue_deployment {
            anyhow::ensure!(
                saved == &deployment,
                "HOLD: queue deployment identity changed"
            );
            return Ok(());
        }
        anyhow::ensure!(
            self.phase == EthereumRootPhase::Rebuilding
                && self.cursor.is_none()
                && self.roots.is_empty(),
            "HOLD: cannot change the replay floor of a progressed legacy root journal"
        );
        self.schema_version = 2;
        self.scan_start = deployment.block_number;
        self.queue_deployment = Some(deployment);
        self.validate()
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            matches!(
                (self.schema_version, self.queue_deployment.as_ref()),
                (1, None) | (2, Some(_))
            ) && self.roots.len() <= ROOT_CAPACITY,
            "HOLD: invalid Ethereum root schema, replay floor or retention bound"
        );
        if let Some(deployment) = &self.queue_deployment {
            deployment.validate()?;
            anyhow::ensure!(
                self.scan_start == deployment.block_number,
                "HOLD: replay floor differs from authenticated queue birth"
            );
        } else {
            anyhow::ensure!(self.scan_start == 0, "HOLD: legacy replay requires genesis");
        }
        anyhow::ensure!(
            self.cursor
                .iter()
                .chain(self.migration_target.iter())
                .all(|pin| pin.block_number >= self.scan_start)
                && self
                    .roots
                    .iter()
                    .all(|root| root.inclusion.block_number >= self.scan_start),
            "HOLD: root evidence precedes authenticated replay floor"
        );
        match (self.phase, self.migration_target) {
            (EthereumRootPhase::Rebuilding, Some(target)) => {
                anyhow::ensure!(
                    self.cursor
                        .is_none_or(|cursor| cursor.block_number <= target.block_number)
                        && self
                            .roots
                            .iter()
                            .all(|root| root.inclusion.block_number <= target.block_number),
                    "HOLD: rebuild state exceeds its immutable migration target"
                );
            }
            (EthereumRootPhase::Ready, None) => {}
            _ => anyhow::bail!("HOLD: inconsistent Ethereum root phase/target"),
        }
        let mut pins = BTreeMap::new();
        if let Some(deployment) = &self.queue_deployment {
            insert_pin(&mut pins, deployment.block_number, deployment.block_hash)?;
        }
        for cursor in self.cursor.iter().chain(self.migration_target.iter()) {
            insert_pin(&mut pins, cursor.block_number, cursor.block_hash)?;
        }
        let mut identities = BTreeMap::new();
        for root in &self.roots {
            insert_pin(
                &mut pins,
                root.inclusion.block_number,
                root.inclusion.block_hash,
            )?;
            anyhow::ensure!(
                root.inclusion.transaction_hash != B256::ZERO,
                "HOLD: Ethereum root lacks a transaction identity"
            );
            let identity = (root.inclusion.block_number, root.inclusion.log_index);
            anyhow::ensure!(
                identities.insert(identity, root).is_none(),
                "HOLD: duplicate or conflicting Ethereum root event identity"
            );
        }
        validate_transaction_identities(self.roots.iter().map(|root| {
            (
                root.inclusion.block_number,
                root.inclusion.transaction_index,
                root.inclusion.transaction_hash,
            )
        }))?;
        let roots = MerkleRoots::from_qualified(&self.roots, ROOT_CAPACITY)
            .context("HOLD: conflicting qualified root state")?;
        anyhow::ensure!(
            roots.qualified_roots() == self.roots,
            "HOLD: noncanonical or duplicate qualified root retention"
        );
        Ok(())
    }

    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        self.validate()?;
        next.validate()?;
        let initial_birth_binding = self.schema_version == 1
            && self.queue_deployment.is_none()
            && self.phase == EthereumRootPhase::Rebuilding
            && self.cursor.is_none()
            && self.roots.is_empty()
            && next.schema_version == 2
            && next.phase == self.phase
            && next.cursor.is_none()
            && next.roots.is_empty()
            && next.migration_target == self.migration_target;
        anyhow::ensure!(
            initial_birth_binding
                || (self.schema_version == next.schema_version
                    && self.scan_start == next.scan_start
                    && self.queue_deployment == next.queue_deployment),
            "HOLD: immutable queue birth/replay identity changed"
        );
        anyhow::ensure!(
            match (self.cursor, next.cursor) {
                (Some(old), Some(new)) => new.block_number > old.block_number || new == old,
                (None, _) => true,
                _ => false,
            },
            "HOLD: Ethereum root cursor changed identity or moved backwards"
        );
        match (self.phase, next.phase) {
            (EthereumRootPhase::Ready, EthereumRootPhase::Ready) => {}
            (EthereumRootPhase::Rebuilding, EthereumRootPhase::Rebuilding) => {
                anyhow::ensure!(
                    self.migration_target == next.migration_target,
                    "HOLD: saved migration target changed"
                );
            }
            (EthereumRootPhase::Rebuilding, EthereumRootPhase::Ready) => {
                anyhow::ensure!(
                    next.cursor == self.migration_target,
                    "HOLD: root replay became ready before its migration target"
                );
            }
            _ => anyhow::bail!("HOLD: qualified root state cannot return to rebuilding"),
        }
        for old in &self.roots {
            if let Some(new) = next.roots.iter().find(|new| {
                new.root.block == old.root.block
                    && new.root.authority_set_id == old.root.authority_set_id
            }) {
                anyhow::ensure!(new == old, "HOLD: retained root provenance changed");
            }
        }
        Ok(())
    }
    fn next_range(&self, head: u64) -> Option<(u64, u64)> {
        let from = match self.cursor {
            Some(cursor) => cursor.block_number.checked_add(1)?,
            None => self.scan_start,
        };
        let to = from.saturating_add(ETH_SCAN_BATCH_BLOCKS - 1).min(head);
        (from <= to).then_some((from, to))
    }

    fn finish_rebuild(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.phase == EthereumRootPhase::Rebuilding && self.cursor == self.migration_target,
            "HOLD: full root replay has not reached its migration target"
        );
        self.phase = EthereumRootPhase::Ready;
        self.migration_target = None;
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait RootCursorStorage: Send + Sync {
    async fn load_root_state(&self) -> Result<Option<EthereumRootState>>;
    async fn save_root_state(&self, state: &EthereumRootState) -> Result<()>;
}

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    confirmations: u64,
    sender: UnboundedSender<RootUpdate>,
    cursor_storage: Arc<dyn RootCursorStorage>,
    state: Option<EthereumRootState>,
    last_finalized: Option<EthereumRootCursor>,
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
            "Latest authenticated finalized Ethereum block",
        ),
    }
}

impl MerkleRootExtractor {
    pub fn new(
        eth_api: EthApi,
        api_provider: ApiProviderConnection,
        confirmations: u64,
        sender: UnboundedSender<RootUpdate>,
        cursor_storage: Arc<dyn RootCursorStorage>,
    ) -> Self {
        Self {
            eth_api,
            api_provider,
            confirmations,
            sender,
            cursor_storage,
            state: None,
            last_finalized: None,
            metrics: Metrics::new(),
        }
    }

    pub fn sender(&self) -> &UnboundedSender<RootUpdate> {
        &self.sender
    }

    pub fn spawn(self) {
        tokio::task::spawn(task(self));
    }

    /// Must finish before consumers are spawned. Legacy numeric cursors and arrays are
    /// evidence only: absence of unified state always starts a complete finalized replay.
    pub async fn qualify(&mut self) -> Result<MerkleRoots> {
        loop {
            match self.qualify_inner().await {
                Ok(roots) => return Ok(roots),
                Err(error) if qualification_holds(&error) => return Err(error),
                Err(error) => {
                    log::warn!("Root qualification unavailable; cursor unchanged: {error:#}");
                    tokio::time::sleep(ETH_RPC_HEALTHCHECK_INTERVAL).await;
                    if let Ok(api) = self.eth_api.reconnect().await {
                        self.eth_api = api;
                    }
                }
            }
        }
    }

    async fn qualify_inner(&mut self) -> Result<MerkleRoots> {
        let mut view = self.finalized_view().await?;
        if self.state.is_none() {
            self.state = self
                .cursor_storage
                .load_root_state()
                .await
                .context("HOLD: cannot load Ethereum root journal")?;
        }
        let saved_birth = self
            .state
            .as_ref()
            .and_then(|state| state.queue_deployment.as_ref());
        let hint = self.eth_api.queue_deployment_transaction();
        if let (Some(saved), Some(hint)) = (saved_birth, hint) {
            anyhow::ensure!(
                hint == saved.transaction_hash,
                "HOLD: queue deployment hint changed"
            );
        }
        let deployment =
            if let Some(hash) = hint.or_else(|| saved_birth.map(|birth| birth.transaction_hash)) {
                Some(
                    self.eth_api
                        .authenticate_queue_deployment(hash, &mut view)
                        .await?,
                )
            } else {
                None
            };
        if self.state.is_none() {
            let Some(head) = confirmed_block(view.block_number(), self.confirmations) else {
                return Err(ethereum_client::Error::FinalizedAncestryPending.into());
            };
            let target = self.cursor_at(head).await?;
            view.verify_blocks(
                self.eth_api.raw_provider(),
                &BTreeMap::from([(target.block_number, target.block_hash)]),
            )
            .await?;
            let mut state = EthereumRootState::rebuilding(target);
            if let Some(deployment) = deployment.clone() {
                state.bind_deployment(deployment)?;
            }
            self.cursor_storage
                .save_root_state(&state)
                .await
                .context("HOLD: cannot initialize root replay")?;
            self.state = Some(state);
        }
        if let Some(deployment) = deployment {
            let state = self.state.as_ref().expect("initialized root state");
            let mut bound = state.clone();
            bound.bind_deployment(deployment)?;
            if &bound != state {
                self.cursor_storage
                    .save_root_state(&bound)
                    .await
                    .context("HOLD: cannot persist authenticated queue birth")?;
                self.state = Some(bound);
            }
        }
        self.authenticate_state(&mut view, true).await?;
        loop {
            let state = self.state.as_ref().expect("initialized root state");
            if state.phase == EthereumRootPhase::Ready {
                // Consumers do not exist yet: hydrate only; normal catch-up owns ACKs.
                return MerkleRoots::from_qualified(&state.roots, ROOT_CAPACITY);
            }
            let head = state
                .migration_target
                .expect("validated target")
                .block_number;
            if let Some((from, to)) = state.next_range(head) {
                self.scan_batch(&mut view, from, to, false).await?;
                continue;
            }
            let mut ready = state.clone();
            ready.finish_rebuild()?;
            self.cursor_storage
                .save_root_state(&ready)
                .await
                .context("HOLD: cannot commit completed root replay")?;
            self.state = Some(ready);
        }
    }

    async fn finalized_view(&mut self) -> Result<FinalizedView> {
        let mut view = tokio::time::timeout(
            ETH_RPC_REQUEST_TIMEOUT,
            self.eth_api.verified_finalized_view(),
        )
        .await
        .context("Ethereum finality request timed out")??;
        if let Some(previous) = self.last_finalized {
            anyhow::ensure!(
                view.block_number() >= previous.block_number,
                "HOLD: finalized Ethereum observation regressed"
            );
            view.verify_blocks(
                self.eth_api.raw_provider(),
                &BTreeMap::from([(previous.block_number, previous.block_hash)]),
            )
            .await?;
        }
        self.last_finalized = Some(EthereumRootCursor {
            block_number: view.block_number(),
            block_hash: view.block_hash(),
        });
        Ok(view)
    }

    async fn cursor_at(&self, number: u64) -> Result<EthereumRootCursor> {
        let block = tokio::time::timeout(
            ETH_RPC_REQUEST_TIMEOUT,
            self.eth_api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(number)),
        )
        .await
        .context("Ethereum header request timed out")??
        .ok_or(ethereum_client::Error::FinalizedAncestryPending)?;
        anyhow::ensure!(
            block.header.number == number && block.header.inner.hash_slow() == block.header.hash,
            "HOLD: malformed Ethereum endpoint header"
        );
        Ok(EthereumRootCursor {
            block_number: number,
            block_hash: block.header.hash,
        })
    }

    async fn authenticate_state(&self, view: &mut FinalizedView, events: bool) -> Result<()> {
        let state = self
            .state
            .as_ref()
            .context("HOLD: root qualification was not initialized")?;
        state.validate()?;
        let mut pins = BTreeMap::new();
        if let Some(deployment) = &state.queue_deployment {
            anyhow::ensure!(
                deployment.genesis_hash == view.genesis_hash()
                    && deployment.queue
                        == alloy::primitives::Address::from(self.eth_api.message_queue_address().0),
                "HOLD: queue birth belongs to another lane/network"
            );
            insert_pin(&mut pins, deployment.block_number, deployment.block_hash)?;
        }
        for cursor in state.cursor.iter().chain(state.migration_target.iter()) {
            insert_pin(&mut pins, cursor.block_number, cursor.block_hash)?;
        }
        for root in &state.roots {
            insert_pin(
                &mut pins,
                root.inclusion.block_number,
                root.inclusion.block_hash,
            )?;
        }
        if pins
            .last_key_value()
            .is_some_and(|(&number, _)| number > view.block_number())
        {
            anyhow::bail!("HOLD: finalized Ethereum view regressed behind retained root evidence");
        }
        view.verify_blocks(self.eth_api.raw_provider(), &pins)
            .await?;
        if events {
            // Bounded by the retained 100 roots, not by the distance between their blocks.
            for (&number, _) in &pins {
                let expected: Vec<_> = state
                    .roots
                    .iter()
                    .filter(|root| root.inclusion.block_number == number)
                    .collect();
                if expected.is_empty() {
                    continue;
                }
                let observations = self.observations(number, number).await?;
                for root in expected {
                    let observation = observations
                        .iter()
                        .find(|observation| observation.log_index == root.inclusion.log_index)
                        .context("HOLD: saved root event is absent from its authenticated block")?;
                    let qualified = self.qualify_observation(observation, view).await?;
                    anyhow::ensure!(
                        qualified == *root,
                        "HOLD: saved root event/source/timestamp identity changed"
                    );
                }
            }
        }
        Ok(())
    }

    async fn observations(&self, from: u64, to: u64) -> Result<Vec<MerkleRootObservation>> {
        let mut observations = tokio::time::timeout(
            ETH_RPC_REQUEST_TIMEOUT,
            self.eth_api.fetch_merkle_roots_in_range(from, to),
        )
        .await
        .context("Ethereum root scan timed out")??;
        let mut identities = BTreeMap::new();
        for observation in &observations {
            anyhow::ensure!(
                !observation.removed && (from..=to).contains(&observation.block_number),
                "HOLD: removed or out-of-range root event"
            );
            u32::try_from(observation.entry.block_number)
                .context("HOLD: source root height exceeds u32")?;
            anyhow::ensure!(
                identities
                    .insert(
                        (observation.block_number, observation.log_index),
                        observation
                    )
                    .is_none(),
                "HOLD: duplicate or conflicting root observation identity"
            );
        }
        validate_transaction_identities(observations.iter().map(|observation| {
            (
                observation.block_number,
                observation.transaction_index,
                observation.transaction_hash,
            )
        }))?;
        observations.sort_by_key(|observation| {
            (
                observation.block_number,
                observation.transaction_index,
                observation.log_index,
            )
        });
        Ok(observations)
    }

    async fn qualify_observation(
        &self,
        observation: &MerkleRootObservation,
        view: &FinalizedView,
    ) -> Result<QualifiedMerkleRoot> {
        anyhow::ensure!(
            !observation.removed && view.contains(observation.block_number, observation.block_hash),
            "HOLD: root event lacks authenticated finalized inclusion"
        );
        let source = u32::try_from(observation.entry.block_number)
            .context("HOLD: source root height exceeds u32")?;
        let queue_root = self.eth_api.read_merkle_root_in_view(source, view).await?;
        anyhow::ensure!(
            queue_root == Some(observation.entry.merkle_root.0),
            "HOLD: observed root differs from finalized queue state"
        );
        let timestamp = self
            .eth_api
            .get_block_timestamp_by_hash(observation.block_number, observation.block_hash)
            .await?;
        let (block_hash, authority_set_id) =
            fetch_hash_auth_id(&self.api_provider.client(), source).await?;
        Ok(QualifiedMerkleRoot {
            root: RelayedMerkleRoot {
                block: GearBlockNumber(source),
                block_hash,
                authority_set_id,
                merkle_root: observation.entry.merkle_root,
                timestamp,
            },
            inclusion: EthereumRootInclusion {
                block_number: observation.block_number,
                block_hash: observation.block_hash,
                transaction_hash: observation.transaction_hash,
                transaction_index: observation.transaction_index,
                log_index: observation.log_index,
            },
        })
    }

    async fn scan_batch(
        &mut self,
        view: &mut FinalizedView,
        from: u64,
        to: u64,
        acknowledge: bool,
    ) -> Result<()> {
        self.authenticate_state(view, false).await?;
        let endpoint = self.cursor_at(to).await?;
        let observations = self.observations(from, to).await?;
        let start = self.cursor_at(from).await?;
        let mut pins = BTreeMap::from([(endpoint.block_number, endpoint.block_hash)]);
        insert_pin(&mut pins, start.block_number, start.block_hash)?;
        for observation in &observations {
            insert_pin(&mut pins, observation.block_number, observation.block_hash)?;
        }
        view.verify_blocks(self.eth_api.raw_provider(), &pins)
            .await?;
        let mut batch = Vec::with_capacity(observations.len());
        for observation in &observations {
            batch.push(self.qualify_observation(observation, view).await?);
        }
        let mut pending = self.state.as_ref().expect("qualified state").clone();
        let mut roots = MerkleRoots::from_qualified(&pending.roots, ROOT_CAPACITY)?;
        for root in &batch {
            anyhow::ensure!(
                !matches!(roots.add(*root), Err(RootAddError::Conflict)),
                "HOLD: conflicting root provenance"
            );
        }
        pending.roots = roots.qualified_roots().to_vec();
        if acknowledge {
            anyhow::ensure!(
                pending.phase == EthereumRootPhase::Ready,
                "HOLD: rebuilding roots cannot enter the accumulator"
            );
            // Save qualified roots with the OLD cursor before exposing them. A crash or
            // lost ACK replays the same range and identical provenance on restart.
            self.cursor_storage
                .save_root_state(&pending)
                .await
                .context("HOLD: cannot persist roots before accumulator acceptance")?;
            self.state = Some(pending.clone());
            for root in batch {
                let (update, accepted) = RootUpdate::tracked(root);
                self.sender
                    .send(update)
                    .map_err(|_| anyhow::anyhow!("Root consumer stopped"))?;
                wait_for_root_acceptance(accepted, ETH_RPC_REQUEST_TIMEOUT).await?;
            }
        }
        // Revalidate the complete batch and prior head after any downstream await.
        let mut fresh = self.finalized_view().await?;
        anyhow::ensure!(
            fresh.block_number() >= view.block_number(),
            "HOLD: finalized Ethereum view regressed during root acceptance"
        );
        insert_pin(&mut pins, view.block_number(), view.block_hash())?;
        fresh
            .verify_blocks(self.eth_api.raw_provider(), &pins)
            .await?;
        self.authenticate_state(&mut fresh, false).await?;
        pending.cursor = Some(endpoint);
        self.cursor_storage
            .save_root_state(&pending)
            .await
            .context("HOLD: cannot atomically commit root scan cursor")?;
        self.state = Some(pending);
        *view = fresh;
        if let Some(root) = observations.last() {
            self.metrics
                .latest_merkle_root_for_block
                .set(i64::try_from(root.entry.block_number)?);
        }
        Ok(())
    }

    async fn catch_up(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.state
                .as_ref()
                .is_some_and(|state| state.phase == EthereumRootPhase::Ready),
            "HOLD: extractor must qualify before spawning consumers"
        );
        let mut view = self.finalized_view().await?;
        self.metrics
            .latest_ethereum_block
            .set(i64::try_from(view.block_number())?);
        self.authenticate_state(&mut view, false).await?;
        let Some(head) = confirmed_block(view.block_number(), self.confirmations) else {
            return Ok(());
        };
        while let Some((from, to)) = self
            .state
            .as_ref()
            .expect("qualified state")
            .next_range(head)
        {
            self.scan_batch(&mut view, from, to, true).await?;
        }
        Ok(())
    }
}

fn validate_transaction_identities(
    identities: impl Iterator<Item = (u64, u64, B256)>,
) -> Result<()> {
    let mut positions = BTreeMap::new();
    let mut transactions = BTreeMap::new();
    for (block, index, hash) in identities {
        anyhow::ensure!(hash != B256::ZERO, "HOLD: missing root transaction hash");
        if let Some(previous) = positions.insert((block, index), hash) {
            anyhow::ensure!(
                previous == hash,
                "HOLD: conflicting transaction hashes at one inclusion index"
            );
        }
        if let Some(previous) = transactions.insert(hash, (block, index)) {
            anyhow::ensure!(
                previous == (block, index),
                "HOLD: root transaction has conflicting inclusion positions"
            );
        }
    }
    Ok(())
}

fn insert_pin(pins: &mut BTreeMap<u64, B256>, number: u64, hash: B256) -> Result<()> {
    anyhow::ensure!(hash != B256::ZERO, "HOLD: missing Ethereum block identity");
    if let Some(previous) = pins.insert(number, hash) {
        anyhow::ensure!(
            previous == hash,
            "HOLD: conflicting Ethereum identities at block {number}"
        );
    }
    Ok(())
}

fn qualification_holds(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.to_string().contains("HOLD")
            || matches!(
                cause.downcast_ref::<ethereum_client::Error>(),
                Some(ethereum_client::Error::FinalizedAncestryConflict)
            )
    })
}

async fn wait_for_root_acceptance(
    accepted: tokio::sync::oneshot::Receiver<Result<()>>,
    timeout: Duration,
) -> Result<()> {
    tokio::time::timeout(timeout, accepted)
        .await
        .context("Timed out waiting for root acceptance")?
        .context("Root consumer stopped before accepting the root")?
}

// The inclusion block counts as confirmation one, matching receipt qualification.
fn confirmed_block(finalized: u64, confirmations: u64) -> Option<u64> {
    finalized.checked_sub(confirmations.saturating_sub(1))
}

async fn fetch_hash_auth_id(gear_api: &GearApi, number: u32) -> Result<(H256, AuthoritySetId)> {
    let hash = gear_api.block_number_to_hash(number).await?;
    let authority = AuthoritySetId(gear_api.signed_by_authority_set_id(hash).await?);
    Ok((hash, authority))
}

async fn task(mut this: MerkleRootExtractor) {
    loop {
        let Err(error) = task_inner(&mut this).await else {
            break;
        };
        log::error!("Merkle root extractor stopped progressing: {error:#}");
        if qualification_holds(&error) {
            break;
        }
        tokio::time::sleep(ETH_RPC_HEALTHCHECK_INTERVAL).await;
        loop {
            match this.eth_api.reconnect().await {
                Ok(api) => {
                    this.eth_api = api;
                    break;
                }
                Err(error) => {
                    log::warn!("Cannot reconnect root extractor: {error}");
                    tokio::time::sleep(ETH_RPC_HEALTHCHECK_INTERVAL).await;
                }
            }
        }
    }
}

async fn task_inner(this: &mut MerkleRootExtractor) -> Result<()> {
    let subscription = this.eth_api.subscribe_logs().await?;
    let mut stream = subscription.into_result_stream();
    let mut interval = tokio::time::interval(ETH_RPC_HEALTHCHECK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        // Subscriptions only wake finalized catch-up; no subscription data is trusted.
        tokio::select! {
            _ = interval.tick() => {}
            event = stream.next() => match event {
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
                None => anyhow::bail!("Ethereum log subscription closed"),
            }
        }
        anyhow::ensure!(
            this.api_provider.is_alive(),
            "Gear API provider is unavailable"
        );
        this.catch_up().await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn deployment() -> QueueDeployment {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            eips::Encodable2718,
            network::TxSigner,
            primitives::{keccak256, TxKind},
            signers::local::PrivateKeySigner,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(17)).unwrap();
        let mut tx = TxEip1559 {
            chain_id: 560048,
            nonce: 31,
            gas_limit: 100_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Create,
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await.unwrap();
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        QueueDeployment {
            chain_id: 560048,
            genesis_hash: B256::repeat_byte(1),
            queue: signer.address().create(31),
            transaction_hash: keccak256(&raw),
            block_number: 9_998,
            block_hash: cursor(9_998).block_hash,
            transaction_index: 0,
            raw_transaction: raw.into(),
        }
    }

    #[tokio::test]
    async fn authenticated_birth_replays_inclusively_and_retains_first_block_across_restart() {
        let birth = deployment().await;
        let legacy = EthereumRootState::rebuilding(cursor(10_000));
        let mut state = legacy.clone();
        state.bind_deployment(birth.clone()).unwrap();
        legacy.validate_successor(&state).unwrap();
        assert_eq!(state.next_range(10_000), Some((9_998, 10_000)));
        state.roots.push(QualifiedMerkleRoot {
            root: RelayedMerkleRoot {
                block: GearBlockNumber(100),
                block_hash: H256::repeat_byte(2),
                timestamp: 1,
                authority_set_id: AuthoritySetId(1),
                merkle_root: H256::repeat_byte(3),
            },
            inclusion: EthereumRootInclusion {
                block_number: birth.block_number,
                block_hash: birth.block_hash,
                transaction_hash: B256::repeat_byte(4),
                transaction_index: 1,
                log_index: 0,
            },
        });
        state.cursor = Some(cursor(10_000));
        state.finish_rebuild().unwrap();
        let restored: EthereumRootState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.roots, state.roots);
        assert_eq!(restored.queue_deployment, Some(birth.clone()));
        assert_eq!(restored.next_range(10_001), Some((10_001, 10_001)));
        let path = std::env::temp_dir().join(format!("queue-birth-{}", uuid::Uuid::new_v4()));
        use crate::message_relayer::gear_to_eth::storage::{tests::lane_identity, JSONStorage};
        let storage = JSONStorage::new(&path);
        let mut lane = lane_identity();
        lane.message_queue_address = primitive_types::H160::from_slice(birth.queue.as_slice());
        storage.bind_outbound_lane(lane).await.unwrap();
        storage
            .save_root_state(&EthereumRootState::rebuilding(cursor(10_000)))
            .await
            .unwrap();
        let mut pending = EthereumRootState::rebuilding(cursor(10_000));
        pending.bind_deployment(birth.clone()).unwrap();
        storage.save_root_state(&pending).await.unwrap();
        storage.save_root_state(&restored).await.unwrap();
        let restarted_storage = JSONStorage::new(&path);
        assert_eq!(
            restarted_storage.load_root_state().await.unwrap().unwrap(),
            restored
        );
        let mut wrong_lane = restored.clone();
        wrong_lane.queue_deployment.as_mut().unwrap().genesis_hash = B256::repeat_byte(44);
        assert!(restarted_storage
            .save_root_state(&wrong_lane)
            .await
            .is_err());
        tokio::fs::remove_dir_all(path).await.unwrap();
        let mut changed = restored.clone();
        changed.scan_start += 1;
        assert!(changed.validate().is_err());
        let mut changed_birth = birth.clone();
        changed_birth.block_hash = B256::repeat_byte(55);
        assert!(changed.bind_deployment(changed_birth).is_err());
        changed = restored.clone();
        changed.queue_deployment.as_mut().unwrap().genesis_hash = B256::repeat_byte(9);
        assert!(restored.validate_successor(&changed).is_err());
        let mut progressed_legacy = legacy;
        progressed_legacy.cursor = Some(cursor(1_999));
        assert!(progressed_legacy.bind_deployment(birth).is_err());
        let mut arbitrary = EthereumRootState::rebuilding(cursor(10_000));
        arbitrary.scan_start = 9_998;
        assert!(arbitrary.validate().is_err());
    }

    fn cursor(number: u64) -> EthereumRootCursor {
        EthereumRootCursor {
            block_number: number,
            block_hash: B256::repeat_byte((number % 254 + 1) as u8),
        }
    }

    #[test]
    fn full_replay_resumes_without_fixed_rewind_and_promotes_only_at_target() {
        let mut state = EthereumRootState::rebuilding(cursor(10_000));
        assert_eq!(state.next_range(10_000), Some((0, 1_999)));
        assert!(state.finish_rebuild().is_err());
        state.cursor = Some(cursor(1_999));
        let saved = serde_json::to_vec(&state).unwrap();
        let mut restored: EthereumRootState = serde_json::from_slice(&saved).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.next_range(10_000), Some((2_000, 3_999)));
        assert!(restored.finish_rebuild().is_err());
        restored.cursor = restored.migration_target;
        restored.finish_rebuild().unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.phase, EthereumRootPhase::Ready);
        assert_eq!(restored.next_range(10_004), Some((10_001, 10_004)));
    }

    #[test]
    fn confirmation_depth_counts_inclusion_and_does_not_saturate_to_genesis() {
        assert_eq!(confirmed_block(6, 8), None);
        assert_eq!(confirmed_block(7, 8), Some(0));
        assert_eq!(confirmed_block(8, 8), Some(1));
        assert_eq!(confirmed_block(8, 0), Some(8));
    }

    #[test]
    fn strict_state_rejects_missing_fields_conflicting_pins_and_wrong_phase() {
        let state = EthereumRootState::rebuilding(cursor(100));
        let mut value = serde_json::to_value(&state).unwrap();
        value.as_object_mut().unwrap().remove("cursor");
        assert!(serde_json::from_value::<EthereumRootState>(value).is_err());
        let mut conflicting = state;
        conflicting.cursor = Some(EthereumRootCursor {
            block_hash: B256::repeat_byte(42),
            ..cursor(100)
        });
        assert!(conflicting.validate().is_err());
        conflicting.cursor = None;
        conflicting.phase = EthereumRootPhase::Ready;
        assert!(conflicting.validate().is_err());
    }

    #[tokio::test]
    async fn stalled_acceptance_leaves_replay_cursor_unchanged() {
        let mut state = EthereumRootState::rebuilding(cursor(105));
        state.cursor = state.migration_target;
        state.finish_rebuild().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<()>>();
        assert!(wait_for_root_acceptance(receiver, Duration::from_millis(1))
            .await
            .is_err());
        drop(sender);
        assert_eq!(state.next_range(110), Some((106, 110)));
    }
}
