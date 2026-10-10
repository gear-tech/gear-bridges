#![allow(dead_code, unused_variables)]
use anyhow::Context;
use primitive_types::{H160, H256, U256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, RwLock},
};
use uuid::Uuid;

use crate::message_relayer::{
    common::{
        ethereum::{accumulator::utils::MerkleRoots, merkle_root_extractor::RootCursorStorage},
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock, GearBlockNumber, MessageInBlock,
    },
    gear_to_eth::tx_manager::{Transaction, TransactionManager, TxStatus},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GearEventStream {
    Queued,
    Paid,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GearEventCursor {
    pub block: u32,
    pub hash: H256,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaidObservation {
    pub nonce: [u8; 32],
    pub block: u32,
    pub block_hash: H256,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PendingEventPair {
    pub message: MessageInBlock,
    pub fee_nonce: [u8; 32],
    pub paid_block: u32,
    pub paid_block_hash: H256,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundLaneIdentity {
    pub destination_chain_id: u64,
    pub destination_genesis_hash: H256,
    pub message_queue_address: H160,
    pub bridging_payment_address: Option<H256>,
    pub fee_exempt_sources: BTreeSet<[u8; 32]>,
    pub sender_address: H160,
}

impl OutboundLaneIdentity {
    fn requires_first_fee(&self, source: &[u8; 32]) -> bool {
        self.bridging_payment_address.is_some() && !self.fee_exempt_sources.contains(source)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedNonce {
    version: u8,
    genesis_hash: H256,
    lane_identity: OutboundLaneIdentity,
    uuid: Uuid,
    message: MessageInBlock,
    // An absent field is lost provenance, not an explicitly fee-exempt message.
    #[serde(deserialize_with = "Option::deserialize")]
    first_paid: Option<PaidObservation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GearEventState {
    version: u8,
    genesis_hash: Option<H256>,
    start_block: Option<u32>,
    #[serde(default)]
    lane_identity: Option<OutboundLaneIdentity>,
    queued_cursor: Option<GearEventCursor>,
    paid_cursor: Option<GearEventCursor>,
    queued: BTreeMap<String, MessageInBlock>,
    paid: BTreeMap<String, PaidObservation>,
    pending_pairs: BTreeMap<String, PendingEventPair>,
}

impl Default for GearEventState {
    fn default() -> Self {
        Self {
            version: 2,
            genesis_hash: None,
            start_block: None,
            lane_identity: None,
            queued_cursor: None,
            paid_cursor: None,
            queued: BTreeMap::new(),
            paid: BTreeMap::new(),
            pending_pairs: BTreeMap::new(),
        }
    }
}

fn check_next_event_cursor(
    last: Option<GearEventCursor>,
    next: GearEventCursor,
) -> anyhow::Result<()> {
    if let Some(last) = last {
        anyhow::ensure!(
            next.block >= last.block,
            "Gear event cursor moved backwards"
        );
        if next.block == last.block {
            anyhow::ensure!(
                next.hash == last.hash,
                "Finalized Gear block hash changed at cursor height"
            );
        } else {
            anyhow::ensure!(
                next.block == last.block.saturating_add(1),
                "Gear event cursor skipped a block"
            );
        }
    }
    Ok(())
}

fn ensure_event_bound(state: &GearEventState, block: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.genesis_hash.is_some() && state.lane_identity.is_some(),
        "HOLD: Gear event journal is not bound to a source/lane identity"
    );
    let start_block = state
        .start_block
        .ok_or_else(|| anyhow::anyhow!("Gear event journal has no start block"))?;
    anyhow::ensure!(
        block >= start_block,
        "Gear event block precedes the configured start"
    );
    Ok(())
}

fn decode_transaction_journal(contents: &str) -> anyhow::Result<Transaction> {
    let mut value: serde_json::Value = serde_json::from_str(contents)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("HOLD: outbound transaction journal is not an object"))?;
    let legacy = !object.contains_key("journal_version");
    if legacy {
        anyhow::ensure!(
            !object.contains_key("ethereum_submission") && !object.contains_key("legacy_evidence"),
            "HOLD: unversioned outbound journal contains new submission fields"
        );
        let original = serde_json::Value::Object(object.clone());
        if let Some(retries) = object.remove("dropped_retries") {
            anyhow::ensure!(
                retries.as_u64().is_some(),
                "HOLD: malformed legacy outbound retry evidence"
            );
        }
        object.insert("journal_version".into(), serde_json::json!(2));
        object.insert("legacy_evidence".into(), original);
    }
    let tx: Transaction = serde_json::from_value(value)?;
    anyhow::ensure!(
        !legacy || !matches!(&tx.status, super::tx_manager::TxStatus::PrepareMessage(..)),
        "HOLD: unversioned outbound journal claims an unsigned handoff guarantee"
    );
    tx.validate_journal()?;
    Ok(tx)
}

async fn transaction_status_view(tx_manager: &TransactionManager) -> serde_json::Value {
    let active: BTreeMap<_, _> = tx_manager
        .transactions
        .read()
        .await
        .iter()
        .map(|(uuid, tx)| {
            let status = match &tx.status {
                TxStatus::WaitForMerkleRoot => "WaitForMerkleRoot",
                TxStatus::FetchMerkleRoot(_) => "FetchMerkleRoot",
                TxStatus::PrepareMessage(..) => "PrepareMessage",
                TxStatus::SendMessage(..) => "SendMessage",
                TxStatus::WaitConfirmations(_) => "WaitConfirmations",
                TxStatus::NeedsReconciliation { .. } => "NeedsReconciliation",
                TxStatus::Failed { .. } => "Failed",
                TxStatus::Completed => "Completed",
            };
            (*uuid, status)
        })
        .collect();
    let failed: Vec<_> = tx_manager.failed.read().await.keys().copied().collect();
    serde_json::json!({"version": 1, "active": active, "failed": failed})
}

pub struct BlockStorage {
    blocks: RwLock<BTreeMap<GearBlockNumber, Block>>,
    save_lock: Mutex<()>,
    n_to_keep: usize,
}

impl Default for BlockStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockStorage {
    pub fn new() -> Self {
        Self {
            blocks: RwLock::new(BTreeMap::new()),
            save_lock: Mutex::new(()),
            n_to_keep: 100,
        }
    }

    pub async fn complete_transaction(&self, message: &MessageInBlock) {
        let mut blocks = self.blocks.write().await;
        let Some(block) = blocks.get_mut(&message.block) else {
            return;
        };

        block
            .messages
            .remove(&U256::from_big_endian(&message.message.nonce_be));
    }

    pub async fn is_message_pending(&self, block: GearBlockNumber, nonce_be: [u8; 32]) -> bool {
        let blocks = self.blocks.read().await;
        let Some(stored_block) = blocks.get(&block) else {
            return false;
        };

        stored_block
            .messages
            .contains(&U256::from_big_endian(&nonce_be))
    }

    pub async fn add_block(
        &self,
        block: GearBlockNumber,
        block_hash: H256,
        txs: impl Iterator<Item = [u8; 32]>,
    ) {
        let mut blocks = self.blocks.write().await;

        if let Some(existing) = blocks.get(&block) {
            if existing.block_hash != block_hash {
                log::warn!(
                    "Block #{} is already in storage with a different hash (existing={}, new={})",
                    block.0,
                    existing.block_hash,
                    block_hash
                );
            }
            // Important: do NOT overwrite existing state.
            // Overwriting would re-introduce already-dequeued messages and cause duplicates.
            return;
        }

        blocks.insert(
            block,
            Block {
                block_hash,
                messages: txs.map(|tx| U256::from_big_endian(&tx)).collect(),
            },
        );
    }

    pub async fn prune(&self) {
        let mut blocks = self.blocks.write().await;

        let mut remove_until = None;

        for (index, (block_number, block)) in blocks.iter().enumerate() {
            if index + self.n_to_keep > blocks.len() {
                remove_until = Some(*block_number);
                break;
            }

            if !block.is_processed() {
                remove_until = Some(*block_number);
                break;
            }
        }

        let Some(remove_until) = remove_until else {
            return;
        };

        *blocks = blocks.split_off(&remove_until);
    }

    pub async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        let blocks = self.blocks.read().await;

        let unprocessed = blocks
            .iter()
            .filter_map(|(block_number, block)| {
                (!block.is_processed()).then_some((block.block_hash, block_number.0))
            })
            .collect::<Vec<_>>();

        let first_block = unprocessed.first().copied();
        let last_block = unprocessed.last().copied();

        UnprocessedBlocks {
            blocks: unprocessed,
            last_block,
            first_block,
        }
    }

    pub async fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let _save_guard = self.save_lock.lock().await;
        let path = path.as_ref();

        let blocks_new = path.join("blocks.json.new");
        let blocks_old = path.join("blocks.json");

        let mut blocks_file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&blocks_new)
            .await
            .with_context(|| {
                format!(
                    "Failed to open or create blocks file in storage path: '{}'",
                    path.display()
                )
            })?;

        self.prune().await;

        let blocks = self.blocks.read().await;
        let blocks_json = serde_json::to_string::<BTreeMap<GearBlockNumber, Block>>(&*blocks)?;

        blocks_file
            .write_all(blocks_json.as_bytes())
            .await
            .with_context(|| {
                format!(
                    "Failed to write blocks to file in storage path: '{}'",
                    path.display()
                )
            })?;

        blocks_file.flush().await?;
        blocks_file.sync_all().await?;
        drop(blocks_file);

        tokio::fs::rename(&blocks_new, &blocks_old)
            .await
            .with_context(|| {
                format!(
                    "Failed to rename new blocks file in storage path: '{}'",
                    path.display()
                )
            })?;

        tokio::fs::File::open(path).await?.sync_all().await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    pub block_hash: H256,
    pub messages: HashSet<U256>,
}

impl Block {
    pub fn is_processed(&self) -> bool {
        self.messages.is_empty()
    }
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    fn block_storage(&self) -> &BlockStorage;

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn save_blocks(&self) -> anyhow::Result<()>;
    async fn message_is_eligible(&self, _message: &MessageInBlock) -> anyhow::Result<bool> {
        anyhow::bail!("Gear event admission is unavailable for this storage")
    }
    async fn outbound_nonce_owned(&self, _nonce: [u8; 32]) -> anyhow::Result<bool> {
        anyhow::bail!("Outbound nonce ownership is unavailable for this storage")
    }
    async fn bind_event_chain(&self, _genesis_hash: H256, _start_block: u32) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn event_start_block(&self) -> anyhow::Result<Option<u32>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn record_queued_block(
        &self,
        _block: u32,
        _hash: H256,
        _messages: &[MessageInBlock],
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn record_paid_block(
        &self,
        _block: u32,
        _hash: H256,
        _nonces: &[[u8; 32]],
    ) -> anyhow::Result<Vec<[u8; 32]>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn event_cursor(
        &self,
        _stream: GearEventStream,
    ) -> anyhow::Result<Option<GearEventCursor>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn queued_observations(&self) -> anyhow::Result<Vec<MessageInBlock>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn paid_observations(&self) -> anyhow::Result<Vec<PaidObservation>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn pending_event_pairs(&self) -> anyhow::Result<Vec<PendingEventPair>> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn ack_event_pair(&self, _nonce: [u8; 32]) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
    async fn replay_from_block(
        &self,
        _configured_start: u32,
        _stream: GearEventStream,
    ) -> anyhow::Result<u32> {
        Err(anyhow::anyhow!(
            "Gear event journal is unavailable for this storage"
        ))
    }
}

#[async_trait::async_trait]
impl<T: Storage> UnprocessedBlocksStorage for T {
    fn requires_finality_proof(&self) -> bool {
        false
    }

    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        self.block_storage().unprocessed_blocks().await
    }

    async fn add_block(
        &self,
        _api: &gear_rpc_client::GearApi,
        _block: &GearBlock,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub struct NoStorage(BlockStorage);

#[cfg(test)]
impl Default for NoStorage {
    fn default() -> Self {
        Self(BlockStorage {
            blocks: RwLock::new(BTreeMap::new()),
            save_lock: Mutex::new(()),
            n_to_keep: 0,
        })
    }
}

#[cfg(test)]
impl NoStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl Storage for NoStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.0
    }

    async fn save(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn message_is_eligible(&self, _message: &MessageInBlock) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn outbound_nonce_owned(&self, _nonce: [u8; 32]) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn ack_event_pair(&self, _nonce: [u8; 32]) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl RootCursorStorage for NoStorage {
    async fn load_eth_cursor(&self) -> anyhow::Result<Option<u64>> {
        Ok(None)
    }
    async fn save_eth_cursor(&self, _block: u64) -> anyhow::Result<()> {
        Ok(())
    }
    async fn save_merkle_roots(&self, _roots: &MerkleRoots) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct JSONStorage {
    path: PathBuf,
    block_storage: BlockStorage,
    save_lock: Mutex<()>,
    merkle_roots_lock: Mutex<()>,
    event_state_lock: Mutex<()>,
    // Runtime-only identity index; completed journals stay in immutable per-nonce files.
    known_completed_nonces: RwLock<HashSet<[u8; 32]>>,
    known_active_nonces: RwLock<BTreeMap<[u8; 32], Uuid>>,
}

impl JSONStorage {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            block_storage: BlockStorage::new(),
            save_lock: Mutex::new(()),
            merkle_roots_lock: Mutex::new(()),
            event_state_lock: Mutex::new(()),
            known_completed_nonces: RwLock::new(HashSet::new()),
            known_active_nonces: RwLock::new(BTreeMap::new()),
        }
    }

    pub async fn bind_outbound_lane(&self, identity: OutboundLaneIdentity) -> anyhow::Result<()> {
        anyhow::ensure!(
            identity.destination_chain_id != 0
                && identity.destination_genesis_hash != H256::zero()
                && identity.message_queue_address != H160::zero()
                && identity.sender_address != H160::zero()
                && identity.bridging_payment_address != Some(H256::zero())
                && (identity.bridging_payment_address.is_some()
                    || identity.fee_exempt_sources.is_empty()),
            "HOLD: incomplete outbound destination/queue/payment/signer identity"
        );
        let _guard = self.event_state_lock.lock().await;
        self.check_incomplete_writes().await?;
        let mut state = self.load_event_state_locked().await?;
        if let Some(existing) = &state.lane_identity {
            anyhow::ensure!(
                existing == &identity,
                "HOLD: outbound lane identity changed"
            );
            return Ok(());
        }
        anyhow::ensure!(
            state.queued_cursor.is_none()
                && state.paid_cursor.is_none()
                && state.queued.is_empty()
                && state.paid.is_empty()
                && state.pending_pairs.is_empty(),
            "HOLD: outbound event cursor or message observations lack a lane identity"
        );
        let mut entries = tokio::fs::read_dir(&self.path).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| Uuid::from_str(name).is_ok())
            {
                anyhow::bail!("HOLD: outbound transaction journal exists without a lane identity");
            }
        }
        let failed_path = self.path.join("failed");
        if failed_path.exists() {
            let failed: BTreeMap<Uuid, String> =
                serde_json::from_slice(&tokio::fs::read(failed_path).await?)?;
            anyhow::ensure!(
                failed.is_empty(),
                "HOLD: failed outbound transactions have no lane identity"
            );
        }
        anyhow::ensure!(
            !self.path.join("ethereum_root_cursor").exists(),
            "HOLD: outbound root cursor lacks a destination lane identity"
        );
        let roots_path = self.path.join("merkle_roots");
        if roots_path.exists() {
            let roots: MerkleRoots = serde_json::from_slice(&tokio::fs::read(roots_path).await?)?;
            anyhow::ensure!(
                roots.is_empty(),
                "HOLD: outbound roots lack a destination lane identity"
            );
        }
        let blocks_path = self.path.join("blocks.json");
        if blocks_path.exists() {
            let blocks: BTreeMap<GearBlockNumber, Block> =
                serde_json::from_slice(&tokio::fs::read(blocks_path).await?)?;
            anyhow::ensure!(
                blocks.is_empty(),
                "HOLD: outbound block cursor lacks a lane identity"
            );
        }
        state.lane_identity = Some(identity);
        self.save_event_state_locked(&state).await
    }

    async fn write_atomic_json<T: Serialize + Sync>(
        &self,
        name: &str,
        value: &T,
    ) -> anyhow::Result<()> {
        let temporary_directory = self.path.join(".state");
        tokio::fs::create_dir_all(&temporary_directory).await?;
        let path = self.path.join(name);
        let temporary_path = temporary_directory.join(format!("{name}.new"));
        let contents = serde_json::to_vec(value)?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .await?;
        file.write_all(&contents).await?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary_path, &path).await?;
        for directory in [&temporary_directory, &self.path] {
            tokio::fs::File::open(directory).await?.sync_all().await?;
        }
        Ok(())
    }

    async fn check_incomplete_writes(&self) -> anyhow::Result<()> {
        let temporary_directory = self.path.join(".state");
        match tokio::fs::read_dir(&temporary_directory).await {
            Ok(mut entries) => {
                if let Some(entry) = entries.next_entry().await? {
                    anyhow::bail!(
                        "Incomplete relayer journal in {}: {} remains in .state",
                        self.path.display(),
                        entry.file_name().to_string_lossy()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        match tokio::fs::symlink_metadata(self.path.join("blocks.json.new")).await {
            Ok(_) => anyhow::bail!("Incomplete relayer journal: blocks.json.new remains"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    async fn load_event_state_locked(&self) -> anyhow::Result<GearEventState> {
        let path = self.path.join("gear_events.json");
        let state = match tokio::fs::read_to_string(path).await {
            Ok(contents) => serde_json::from_str::<GearEventState>(&contents)
                .context("Invalid Gear event journal")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir_all(&self.path).await?;
                let mut entries = tokio::fs::read_dir(&self.path).await?;
                anyhow::ensure!(
                    entries.next_entry().await?.is_none(),
                    "Existing relayer storage is missing gear_events.json; refusing to resume it"
                );
                GearEventState::default()
            }
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
            state.version == 2,
            "Unsupported Gear event journal schema {}",
            state.version
        );
        Ok(state)
    }

    async fn save_event_state_locked(&self, state: &GearEventState) -> anyhow::Result<()> {
        self.write_atomic_json("gear_events.json", state).await
    }

    fn pair_event(state: &mut GearEventState, nonce: &str) {
        if state.pending_pairs.contains_key(nonce) {
            return;
        }
        let pair = state
            .queued
            .get(nonce)
            .zip(state.paid.get(nonce))
            .map(|(message, paid)| PendingEventPair {
                message: message.clone(),
                fee_nonce: paid.nonce,
                paid_block: paid.block,
                paid_block_hash: paid.block_hash,
            });
        if let Some(pair) = pair {
            state.pending_pairs.insert(nonce.to_owned(), pair);
        }
    }

    fn is_manual_event_state(state: &GearEventState) -> bool {
        state
            .lane_identity
            .as_ref()
            .is_some_and(|lane| lane.bridging_payment_address.is_none())
            && state.queued_cursor.is_none()
            && state.paid_cursor.is_none()
            && state.queued.is_empty()
            && state.paid.is_empty()
            && state.pending_pairs.is_empty()
    }

    fn retained_first_fee(
        state: &GearEventState,
        message: &MessageInBlock,
    ) -> anyhow::Result<Option<PaidObservation>> {
        let key = hex::encode(message.message.nonce_be);
        ensure_event_bound(state, message.block.0)?;
        anyhow::ensure!(
            state.queued.get(&key) == Some(message),
            "HOLD: original queued evidence is missing or conflicting for nonce {key}"
        );
        let cursor = state
            .queued_cursor
            .ok_or_else(|| anyhow::anyhow!("HOLD: original queued cursor is missing"))?;
        anyhow::ensure!(
            message.block.0 <= cursor.block
                && (message.block.0 != cursor.block || message.block_hash == cursor.hash),
            "HOLD: original queued evidence exceeds or conflicts with its cursor"
        );
        let paid = state.paid.get(&key).cloned();
        let lane = state.lane_identity.as_ref().expect("event lane is bound");
        anyhow::ensure!(
            paid.is_some() || !lane.requires_first_fee(&message.message.source),
            "HOLD: original non-exempt outbound message is missing its first fee"
        );
        if let Some(paid) = &paid {
            ensure_event_bound(state, paid.block)?;
            let cursor = state
                .paid_cursor
                .ok_or_else(|| anyhow::anyhow!("HOLD: original paid cursor is missing"))?;
            anyhow::ensure!(
                paid.nonce == message.message.nonce_be
                    && paid.block <= cursor.block
                    && (paid.block != cursor.block || paid.block_hash == cursor.hash),
                "HOLD: original fee evidence conflicts with its nonce/cursor"
            );
        }
        anyhow::ensure!(
            match (paid.as_ref(), state.pending_pairs.get(&key)) {
                (Some(paid), Some(pair)) =>
                    &pair.message == message
                        && pair.fee_nonce == paid.nonce
                        && pair.paid_block == paid.block
                        && pair.paid_block_hash == paid.block_hash,
                (None, None) => true,
                _ => false,
            },
            "HOLD: original queued/paid pair evidence is missing or conflicting for nonce {key}"
        );
        Ok(paid)
    }

    fn completed_nonce_file(nonce: [u8; 32]) -> String {
        format!("completed_nonce_{}.json", hex::encode(nonce))
    }

    async fn completed_nonce_locked(
        &self,
        state: &GearEventState,
        nonce: [u8; 32],
    ) -> anyhow::Result<Option<CompletedNonce>> {
        let bytes = match tokio::fs::read(self.path.join(Self::completed_nonce_file(nonce))).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(
                    !self.known_completed_nonces.read().await.contains(&nonce),
                    "HOLD: known completed nonce is missing immutable ownership evidence"
                );
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let owner: CompletedNonce =
            serde_json::from_slice(&bytes).context("HOLD: invalid completed nonce evidence")?;
        anyhow::ensure!(
            owner.version == 1
                && Some(owner.genesis_hash) == state.genesis_hash
                && state.lane_identity.as_ref() == Some(&owner.lane_identity)
                && owner.message.message.nonce_be == nonce,
            "HOLD: completed nonce evidence changed source/lane/message identity"
        );
        ensure_event_bound(state, owner.message.block.0)?;
        let queued_cursor = state.queued_cursor.ok_or_else(|| {
            anyhow::anyhow!("HOLD: completed nonce has no original queued cursor")
        })?;
        anyhow::ensure!(
            owner.message.block.0 <= queued_cursor.block
                && (owner.message.block.0 != queued_cursor.block
                    || owner.message.block_hash == queued_cursor.hash),
            "HOLD: completed nonce original queued inclusion conflicts with its cursor"
        );
        anyhow::ensure!(
            owner.first_paid.is_some()
                || !owner
                    .lane_identity
                    .requires_first_fee(&owner.message.message.source),
            "HOLD: completed non-exempt outbound nonce is missing its original first fee"
        );
        if let Some(paid) = &owner.first_paid {
            ensure_event_bound(state, paid.block)?;
            let cursor = state.paid_cursor.ok_or_else(|| {
                anyhow::anyhow!("HOLD: completed nonce has no original paid cursor")
            })?;
            anyhow::ensure!(
                paid.nonce == nonce
                    && paid.block <= cursor.block
                    && (paid.block != cursor.block || paid.block_hash == cursor.hash),
                "HOLD: completed nonce original fee conflicts with its nonce/cursor"
            );
        }
        let key = hex::encode(nonce);
        if state.queued.contains_key(&key)
            || state.paid.contains_key(&key)
            || state.pending_pairs.contains_key(&key)
        {
            anyhow::ensure!(
                Self::retained_first_fee(state, &owner.message)? == owner.first_paid,
                "HOLD: completed nonce first fee provenance changed"
            );
        }
        let contents = tokio::fs::read_to_string(self.path.join(owner.uuid.to_string()))
            .await
            .context("HOLD: completed nonce is missing its original UUID journal")?;
        let tx = decode_transaction_journal(&contents)?;
        anyhow::ensure!(
            tx.uuid == owner.uuid
                && tx.message == owner.message
                && matches!(&tx.status, TxStatus::Completed),
            "HOLD: completed nonce ownership conflicts with its original UUID journal"
        );
        self.known_completed_nonces.write().await.insert(nonce);
        Ok(Some(owner))
    }

    async fn validate_event_transactions(
        &self,
        tx_manager: &TransactionManager,
    ) -> anyhow::Result<()> {
        let _guard = self.event_state_lock.lock().await;
        let state = self.load_event_state_locked().await?;
        let active = tx_manager.transactions.read().await;
        let completed = tx_manager.completed.read().await;
        let mut nonces = HashSet::new();
        for tx in active.values().chain(completed.values()) {
            let nonce = tx.message.message.nonce_be;
            anyhow::ensure!(
                nonces.insert(nonce),
                "HOLD: multiple outbound UUIDs claim nonce {}",
                hex::encode(nonce)
            );
            ensure_event_bound(&state, tx.message.block.0)?;
            if matches!(&tx.status, TxStatus::Completed) && !Self::is_manual_event_state(&state) {
                self.known_completed_nonces.write().await.insert(nonce);
            }
            let owner = self.completed_nonce_locked(&state, nonce).await?;
            if let Some(owner) = owner {
                anyhow::ensure!(
                    owner.uuid == tx.uuid
                        && owner.message == tx.message
                        && matches!(&tx.status, TxStatus::Completed),
                    "HOLD: outbound UUID conflicts with completed nonce ownership"
                );
            } else if !Self::is_manual_event_state(&state) {
                anyhow::ensure!(
                    !matches!(&tx.status, TxStatus::Completed),
                    "HOLD: completed outbound nonce {} lacks immutable original fee/UUID evidence",
                    hex::encode(nonce)
                );
                Self::retained_first_fee(&state, &tx.message)?;
            }
        }
        Ok(())
    }

    async fn write_tx(&self, tx_uuid: &Uuid, tx: &Transaction) -> anyhow::Result<()> {
        tx.validate_journal()?;
        anyhow::ensure!(
            *tx_uuid == tx.uuid,
            "HOLD: outbound transaction UUID changed"
        );
        self.validate_submission_lane(tx).await?;
        let _guard = self.event_state_lock.lock().await;
        let mut state = self.load_event_state_locked().await?;
        ensure_event_bound(&state, tx.message.block.0)?;
        let nonce = tx.message.message.nonce_be;
        let existing = self.completed_nonce_locked(&state, nonce).await?;
        if let Some(owner) = &existing {
            anyhow::ensure!(
                owner.uuid == tx.uuid
                    && owner.message == tx.message
                    && matches!(&tx.status, TxStatus::Completed),
                "HOLD: outbound transaction conflicts with completed nonce ownership"
            );
        }
        let new_owner = if existing.is_none() && !Self::is_manual_event_state(&state) {
            let first_paid = Self::retained_first_fee(&state, &tx.message)?;
            matches!(&tx.status, TxStatus::Completed).then(|| CompletedNonce {
                version: 1,
                genesis_hash: state.genesis_hash.expect("event source is bound"),
                lane_identity: state.lane_identity.take().expect("event lane is bound"),
                uuid: tx.uuid,
                message: state
                    .queued
                    .remove(&hex::encode(nonce))
                    .expect("original queued evidence is retained"),
                first_paid,
            })
        } else {
            None
        };
        self.write_atomic_json(&tx_uuid.to_string(), tx).await?;
        if !matches!(&tx.status, TxStatus::Completed) {
            self.known_active_nonces
                .write()
                .await
                .insert(nonce, *tx_uuid);
        }
        if let Some(owner) = new_owner {
            // Losing ownership after the completed UUID save must HOLD, including failed sidecar writes.
            self.known_completed_nonces.write().await.insert(nonce);
            // The UUID journal is durable before ownership can suppress discovery or acknowledgement.
            self.write_atomic_json(&Self::completed_nonce_file(nonce), &owner)
                .await?;
        }
        if matches!(&tx.status, TxStatus::Completed) {
            self.known_active_nonces.write().await.remove(&nonce);
        }
        Ok(())
    }

    async fn validate_submission_lane(&self, tx: &Transaction) -> anyhow::Result<()> {
        let Some(signed) = &tx.ethereum_submission else {
            return Ok(());
        };
        let _guard = self.event_state_lock.lock().await;
        let state = self.load_event_state_locked().await?;
        let lane = state.lane_identity.as_ref().ok_or_else(|| {
            anyhow::anyhow!("HOLD: signed outbound journal has no destination lane identity")
        })?;
        anyhow::ensure!(
            signed.chain_id == lane.destination_chain_id
                && signed.contract.as_slice() == lane.message_queue_address.as_bytes()
                && signed.sender.as_slice() == lane.sender_address.as_bytes(),
            "HOLD: original signed outbound transaction changed chain/queue/signer lane"
        );
        Ok(())
    }

    async fn read_tx(&self, path: PathBuf, tx_file: OsString) -> anyhow::Result<Transaction> {
        let uuid = tx_file
            .to_str()
            .and_then(|s| Uuid::from_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in file name: {tx_file:?}"))?;

        let mut contents = String::new();

        let mut file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("Failed to open transaction file: {tx_file:?}"))?;

        file.read_to_string(&mut contents)
            .await
            .with_context(|| format!("Failed to read transaction file: {tx_file:?}"))?;

        let tx = decode_transaction_journal(&contents)
            .with_context(|| format!("Failed to deserialize transaction from file: {tx_file:?}"))?;
        self.validate_submission_lane(&tx).await?;

        if tx.uuid != uuid {
            return Err(anyhow::anyhow!(
                "Transaction UUID mismatch: expected {}, found {}",
                uuid,
                tx.uuid
            ));
        }

        Ok(tx)
    }
}

#[async_trait::async_trait]
impl Storage for JSONStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.block_storage
    }

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        let _save_guard = self.save_lock.lock().await;
        if !self.path.exists() {
            tokio::fs::create_dir_all(&self.path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create storage directory: {}",
                        self.path.display()
                    )
                })?;
        }

        let temporary_directory = self.path.join(".state");
        tokio::fs::create_dir_all(&temporary_directory).await?;
        let marker = temporary_directory.join("save.pending");
        let marker_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .await
            .context("An interrupted relayer journal save must be reconciled before continuing")?;
        marker_file.sync_all().await?;
        drop(marker_file);
        tokio::fs::File::open(&temporary_directory)
            .await?
            .sync_all()
            .await?;

        let mut persisted_transactions = HashSet::new();

        for (tx_uuid, tx) in tx_manager.transactions.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
            persisted_transactions.insert(*tx_uuid);
        }

        for (tx_uuid, tx) in tx_manager.completed.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
            persisted_transactions.insert(*tx_uuid);
        }

        let failed = tx_manager.failed.read().await;
        self.write_atomic_json("failed", &*failed).await?;

        let _guard = self.merkle_roots_lock.lock().await;
        let merkle = tx_manager.merkle_roots.read().await;
        self.write_atomic_json("merkle_roots", &*merkle).await?;

        self.block_storage.save(&self.path).await?;
        self.write_atomic_json(
            "transaction_status.json",
            &transaction_status_view(tx_manager).await,
        )
        .await?;

        // Remove transaction files that are no longer active or completed only
        // after every current state file has been saved successfully. Without
        // this cleanup, a failed transaction is resurrected on the next load.
        let mut entries = tokio::fs::read_dir(&self.path).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }

            let Some(uuid) = entry
                .file_name()
                .to_str()
                .and_then(|name| Uuid::from_str(name).ok())
            else {
                continue;
            };

            if !persisted_transactions.contains(&uuid) {
                tokio::fs::remove_file(entry.path())
                    .await
                    .with_context(|| {
                        format!("Failed to remove stale transaction file for {uuid}")
                    })?;
            }
        }

        tokio::fs::File::open(&self.path).await?.sync_all().await?;
        tokio::fs::remove_file(marker).await?;
        tokio::fs::File::open(&temporary_directory)
            .await?
            .sync_all()
            .await?;
        Ok(())
    }

    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        self.check_incomplete_writes().await?;

        // Load failures first: legacy snapshots can contain both a UUID transaction
        // file and the same UUID in `failed`. The failure record is authoritative,
        // otherwise directory iteration order could resurrect a terminal transaction.
        let failed_path = self.path.join("failed");
        if failed_path.exists() {
            let contents = tokio::fs::read_to_string(&failed_path)
                .await
                .context("Failed to read 'failed' transactions file")?;
            let map: BTreeMap<Uuid, String> =
                serde_json::from_str(&contents).context("Failed to parse 'failed' transactions")?;
            tx_manager.failed.write().await.extend(map);
        }

        let mut journaled_failures = HashSet::new();
        let mut dir = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = dir.next_entry().await? {
            if entry
                .file_type()
                .await
                .context("directory entry is unaccessible")?
                .is_file()
            {
                if entry.file_name().to_str() == Some("failed") {
                    // Loaded before scanning transaction files.
                    continue;
                } else if entry.file_name().to_str() == Some("merkle_roots") {
                    let contents = tokio::fs::read_to_string(entry.path())
                        .await
                        .context("Failed to read 'merkle_roots' file")?;
                    let merkle_roots: MerkleRoots = serde_json::from_str(&contents)
                        .context("Failed to parse 'merkle_roots'")?;
                    for i in 0..merkle_roots.len() {
                        let root = merkle_roots.get(i).expect("Root should exist");
                        let _ = tx_manager.merkle_roots.write().await.add(*root);
                    }
                } else if entry.file_name().to_str() == Some("blocks.json") {
                    let contents =
                        tokio::fs::read_to_string(entry.path())
                            .await
                            .with_context(|| {
                                format!(
                                    "Failed to read blocks file in storage path: '{}'",
                                    self.path.display()
                                )
                            })?;
                    let map: BTreeMap<GearBlockNumber, Block> = serde_json::from_str(&contents)?;
                    *self.block_storage.blocks.write().await = map;
                } else if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("completed_nonce_"))
                {
                    let name = entry.file_name();
                    let name = name.to_str().expect("completed nonce file name is UTF-8");
                    let key = name
                        .strip_prefix("completed_nonce_")
                        .and_then(|key| key.strip_suffix(".json"))
                        .ok_or_else(|| {
                            anyhow::anyhow!("HOLD: invalid completed nonce file name")
                        })?;
                    let nonce: [u8; 32] = hex::decode(key)?
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("HOLD: invalid completed nonce key"))?;
                    anyhow::ensure!(
                        name == Self::completed_nonce_file(nonce),
                        "HOLD: noncanonical completed nonce file name"
                    );
                    let _guard = self.event_state_lock.lock().await;
                    let state = self.load_event_state_locked().await?;
                    self.completed_nonce_locked(&state, nonce)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("HOLD: missing completed nonce evidence"))?;
                } else if matches!(
                    entry.file_name().to_str(),
                    Some("ethereum_root_cursor" | "gear_events.json" | "transaction_status.json")
                ) {
                    continue;
                } else {
                    let tx = self.read_tx(entry.path(), entry.file_name()).await?;

                    if let Some(reason) = tx_manager.failed.read().await.get(&tx.uuid) {
                        anyhow::ensure!(
                            matches!(&tx.status, super::tx_manager::TxStatus::Failed { diagnostic } if diagnostic == reason),
                            "HOLD: legacy failed transaction {} lacks its full terminal journal",
                            tx.uuid
                        );
                        journaled_failures.insert(tx.uuid);
                    }

                    tx_manager.add_transaction(tx).await;
                }
            }
        }
        for uuid in tx_manager.failed.read().await.keys() {
            anyhow::ensure!(
                journaled_failures.contains(uuid),
                "HOLD: failed outbound transaction {uuid} has no full message/attempt journal"
            );
        }
        match tokio::fs::read(self.path.join("transaction_status.json")).await {
            Ok(bytes) => {
                let view: serde_json::Value = serde_json::from_slice(&bytes)
                    .context("HOLD: malformed outbound transaction status view")?;
                anyhow::ensure!(
                    view == transaction_status_view(tx_manager).await,
                    "HOLD: outbound transaction status view disagrees with original journals"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.validate_event_transactions(tx_manager).await?;
        *self.known_active_nonces.write().await = tx_manager
            .transactions
            .read()
            .await
            .values()
            .map(|tx| (tx.message.message.nonce_be, tx.uuid))
            .collect();
        Ok(())
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        self.block_storage.save(&self.path).await
    }

    async fn message_is_eligible(&self, message: &MessageInBlock) -> anyhow::Result<bool> {
        let _guard = self.event_state_lock.lock().await;
        let state = self.load_event_state_locked().await?;
        let start = state
            .start_block
            .ok_or_else(|| anyhow::anyhow!("HOLD: Gear event journal has no start block"))?;
        ensure_event_bound(&state, start)?;
        if message.block.0 < start {
            return Ok(false);
        }
        // Only the isolated, explicitly no-payment manual journal has no event cursors.
        if Self::is_manual_event_state(&state) {
            return Ok(true);
        }
        let key = hex::encode(message.message.nonce_be);
        let Some(queued) = state.queued.get(&key) else {
            return Ok(false);
        };
        anyhow::ensure!(
            queued == message,
            "HOLD: requested message conflicts with original queued evidence"
        );
        let lane = state.lane_identity.as_ref().expect("event lane is bound");
        if lane.requires_first_fee(&message.message.source) && !state.paid.contains_key(&key) {
            return Ok(false);
        }
        Self::retained_first_fee(&state, message)?;
        Ok(true)
    }

    async fn outbound_nonce_owned(&self, nonce: [u8; 32]) -> anyhow::Result<bool> {
        let _guard = self.event_state_lock.lock().await;
        let uuid = self.known_active_nonces.read().await.get(&nonce).copied();
        // Restore and durable writes populate these indexes. Unowned paid-first
        // nonces need no repeated parse of the entire event journal.
        if uuid.is_none() && !self.known_completed_nonces.read().await.contains(&nonce) {
            return Ok(false);
        }
        let state = self.load_event_state_locked().await?;
        if self.completed_nonce_locked(&state, nonce).await?.is_some() {
            return Ok(true);
        }
        let Some(uuid) = uuid else {
            return Ok(false);
        };
        let contents = tokio::fs::read_to_string(self.path.join(uuid.to_string()))
            .await
            .context("HOLD: active outbound nonce is missing its original UUID journal")?;
        let tx = decode_transaction_journal(&contents)?;
        anyhow::ensure!(
            tx.uuid == uuid && tx.message.message.nonce_be == nonce,
            "HOLD: active outbound nonce ownership changed"
        );
        if !Self::is_manual_event_state(&state) {
            Self::retained_first_fee(&state, &tx.message)?;
        }
        Ok(true)
    }

    async fn bind_event_chain(&self, genesis_hash: H256, start_block: u32) -> anyhow::Result<()> {
        let _guard = self.event_state_lock.lock().await;
        self.check_incomplete_writes().await?;
        let mut state = self.load_event_state_locked().await?;
        anyhow::ensure!(
            state.lane_identity.is_some(),
            "HOLD: bind outbound lane identity before source event replay"
        );
        match (state.genesis_hash, state.start_block) {
            (Some(existing_genesis), Some(existing_start)) => {
                anyhow::ensure!(
                    existing_genesis == genesis_hash,
                    "Gear event journal belongs to another genesis"
                );
                anyhow::ensure!(
                    existing_start == start_block,
                    "Gear event journal start block changed"
                );
                Ok(())
            }
            (None, None) => {
                state.genesis_hash = Some(genesis_hash);
                state.start_block = Some(start_block);
                self.save_event_state_locked(&state).await
            }
            _ => anyhow::bail!("Gear event journal has incomplete chain identity"),
        }
    }

    async fn record_queued_block(
        &self,
        block: u32,
        hash: H256,
        messages: &[MessageInBlock],
    ) -> anyhow::Result<()> {
        let _guard = self.event_state_lock.lock().await;
        let mut state = self.load_event_state_locked().await?;
        ensure_event_bound(&state, block)?;
        let cursor = GearEventCursor { block, hash };
        check_next_event_cursor(state.queued_cursor, cursor)?;
        let replayed = state.queued_cursor == Some(cursor);
        for message in messages {
            anyhow::ensure!(
                message.block.0 == block && message.block_hash == hash,
                "Queued message identity does not match its finalized block"
            );
            let nonce = hex::encode(message.message.nonce_be);
            if let Some(owner) = self
                .completed_nonce_locked(&state, message.message.nonce_be)
                .await?
            {
                anyhow::ensure!(
                    owner.message == *message,
                    "HOLD: conflicting queued event for completed nonce {nonce}"
                );
                continue;
            }
            if let Some(existing) = state.queued.get(&nonce) {
                anyhow::ensure!(
                    existing == message,
                    "Conflicting queued event for message nonce {nonce}"
                );
            } else if !replayed {
                state.queued.insert(nonce.clone(), message.clone());
            }
            if !replayed {
                Self::pair_event(&mut state, &nonce);
            }
        }
        if replayed {
            return Ok(());
        }
        state.queued_cursor = Some(cursor);
        self.save_event_state_locked(&state).await
    }

    async fn record_paid_block(
        &self,
        block: u32,
        hash: H256,
        nonces: &[[u8; 32]],
    ) -> anyhow::Result<Vec<[u8; 32]>> {
        let _guard = self.event_state_lock.lock().await;
        let mut state = self.load_event_state_locked().await?;
        ensure_event_bound(&state, block)?;
        let cursor = GearEventCursor { block, hash };
        check_next_event_cursor(state.paid_cursor, cursor)?;
        let mut discovered = Vec::new();
        if state.paid_cursor == Some(cursor) {
            return Ok(discovered);
        }
        for nonce in nonces {
            if self.completed_nonce_locked(&state, *nonce).await?.is_some() {
                continue;
            }
            let key = hex::encode(nonce);
            if !state.paid.contains_key(&key) {
                state.paid.insert(
                    key.clone(),
                    PaidObservation {
                        nonce: *nonce,
                        block,
                        block_hash: hash,
                    },
                );
                discovered.push(*nonce);
            }
            Self::pair_event(&mut state, &key);
        }
        state.paid_cursor = Some(cursor);
        self.save_event_state_locked(&state).await?;
        Ok(discovered)
    }

    async fn event_start_block(&self) -> anyhow::Result<Option<u32>> {
        let _guard = self.event_state_lock.lock().await;
        Ok(self.load_event_state_locked().await?.start_block)
    }

    async fn event_cursor(
        &self,
        stream: GearEventStream,
    ) -> anyhow::Result<Option<GearEventCursor>> {
        let _guard = self.event_state_lock.lock().await;
        let state = self.load_event_state_locked().await?;
        Ok(match stream {
            GearEventStream::Queued => state.queued_cursor,
            GearEventStream::Paid => state.paid_cursor,
        })
    }

    async fn queued_observations(&self) -> anyhow::Result<Vec<MessageInBlock>> {
        let _guard = self.event_state_lock.lock().await;
        Ok(self
            .load_event_state_locked()
            .await?
            .queued
            .into_values()
            .collect())
    }

    async fn paid_observations(&self) -> anyhow::Result<Vec<PaidObservation>> {
        let _guard = self.event_state_lock.lock().await;
        Ok(self
            .load_event_state_locked()
            .await?
            .paid
            .into_values()
            .collect())
    }

    async fn pending_event_pairs(&self) -> anyhow::Result<Vec<PendingEventPair>> {
        let _guard = self.event_state_lock.lock().await;
        Ok(self
            .load_event_state_locked()
            .await?
            .pending_pairs
            .into_values()
            .collect())
    }

    async fn ack_event_pair(&self, nonce: [u8; 32]) -> anyhow::Result<()> {
        let _guard = self.event_state_lock.lock().await;
        let mut state = self.load_event_state_locked().await?;
        let owner = self.completed_nonce_locked(&state, nonce).await?;
        if owner.is_none() && Self::is_manual_event_state(&state) {
            return Ok(());
        }
        anyhow::ensure!(
            owner.is_some(),
            "HOLD: nonce {} has no durable completed ownership",
            hex::encode(nonce)
        );
        let key = hex::encode(nonce);
        if !state.queued.contains_key(&key)
            && !state.paid.contains_key(&key)
            && !state.pending_pairs.contains_key(&key)
        {
            return Ok(());
        }
        state.pending_pairs.remove(&key);
        state.queued.remove(&key);
        state.paid.remove(&key);
        self.save_event_state_locked(&state).await
    }

    async fn replay_from_block(
        &self,
        configured_start: u32,
        stream: GearEventStream,
    ) -> anyhow::Result<u32> {
        let _guard = self.event_state_lock.lock().await;
        let state = self.load_event_state_locked().await?;
        let start = state.start_block.unwrap_or(configured_start);
        let cursor = match stream {
            GearEventStream::Queued => state.queued_cursor,
            GearEventStream::Paid => state.paid_cursor,
        };
        Ok(cursor.map_or(start, |cursor| cursor.block.saturating_add(1)))
    }
}
#[async_trait::async_trait]
impl RootCursorStorage for JSONStorage {
    async fn load_eth_cursor(&self) -> anyhow::Result<Option<u64>> {
        let path = self.path.join("ethereum_root_cursor");
        match tokio::fs::read_to_string(path).await {
            Ok(contents) => Ok(Some(
                serde_json::from_str(&contents).context("Invalid Ethereum root cursor")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn save_eth_cursor(&self, block: u64) -> anyhow::Result<()> {
        let _guard = self.merkle_roots_lock.lock().await;
        self.write_atomic_json("ethereum_root_cursor", &block).await
    }

    async fn save_merkle_roots(&self, roots: &MerkleRoots) -> anyhow::Result<()> {
        let _guard = self.merkle_roots_lock.lock().await;
        self.write_atomic_json("merkle_roots", roots).await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::message_relayer::common::{AuthoritySetId, GearBlockNumber, MessageInBlock};
    use std::sync::Arc;

    fn msg_in_block(block: u32, nonce: u64) -> MessageInBlock {
        let nonce_be = U256::from(nonce).to_big_endian();

        MessageInBlock {
            message: gear_rpc_client::dto::Message {
                nonce_be,
                source: [1u8; 32],
                destination: [2u8; 20],
                payload: vec![0xAA, 0xBB],
            },
            block: GearBlockNumber(block),
            block_hash: H256::from_low_u64_be(123),
            authority_set_id: AuthoritySetId(1),
        }
    }

    pub(crate) fn lane_identity() -> OutboundLaneIdentity {
        OutboundLaneIdentity {
            destination_chain_id: 560048,
            destination_genesis_hash: H256::repeat_byte(1),
            message_queue_address: H160::repeat_byte(2),
            bridging_payment_address: Some(H256::repeat_byte(3)),
            fee_exempt_sources: BTreeSet::new(),
            sender_address: H160::from_slice(
                alloy::signers::local::PrivateKeySigner::from_bytes(
                    &alloy::primitives::B256::from([7; 32]),
                )
                .unwrap()
                .address()
                .as_slice(),
            ),
        }
    }

    pub(crate) async fn completed_tx(message: MessageInBlock) -> Transaction {
        use crate::message_relayer::common::ethereum::status_fetcher::tests::{
            evidence, signed_message,
        };
        let signed = signed_message(&message.message).await;
        let completion = evidence(&message.message, &signed, false);
        let mut tx = Transaction::new(message, TxStatus::Completed);
        tx.ethereum_tx_attempts.push(signed.hash);
        tx.ethereum_tx_hash = Some(signed.hash);
        tx.ethereum_submission = Some(signed);
        tx.completion = Some(completion);
        tx
    }

    fn manual_lane_identity() -> OutboundLaneIdentity {
        OutboundLaneIdentity {
            bridging_payment_address: None,
            ..lane_identity()
        }
    }

    async fn paired_storage(path: &Path) -> (Arc<JSONStorage>, MessageInBlock, PaidObservation) {
        let storage = Arc::new(JSONStorage::new(path));
        storage.bind_outbound_lane(lane_identity()).await.unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 10)
            .await
            .unwrap();
        let mut message = msg_in_block(10, 7);
        message.block_hash = H256::repeat_byte(10);
        storage
            .record_queued_block(10, message.block_hash, &[message.clone()])
            .await
            .unwrap();
        storage
            .record_paid_block(10, message.block_hash, &[])
            .await
            .unwrap();
        let paid = PaidObservation {
            nonce: message.message.nonce_be,
            block: 11,
            block_hash: H256::repeat_byte(11),
        };
        storage
            .record_paid_block(paid.block, paid.block_hash, &[paid.nonce])
            .await
            .unwrap();
        (storage, message, paid)
    }

    #[tokio::test]
    async fn missing_completed_nonce_sidecar_holds_ongoing_discovery() {
        for interruption in ["saved", "restarted", "sidecar_write_failure"] {
            let path =
                std::env::temp_dir().join(format!("gear-live-missing-owner-{}", Uuid::new_v4()));
            let (storage, message, _) = paired_storage(&path).await;
            let manager = TransactionManager::new(storage.clone());
            let tx = completed_tx(message.clone()).await;
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            let owner_file = JSONStorage::completed_nonce_file(message.message.nonce_be);
            let running_storage = if interruption == "sidecar_write_failure" {
                tokio::fs::write(
                    path.join(".state").join(format!("{owner_file}.new")),
                    b"{interrupted",
                )
                .await
                .unwrap();
                assert!(manager.update_storage().await.is_err());
                assert!(!path.join(&owner_file).exists());
                assert!(path.join(".state/save.pending").exists());
                storage.clone()
            } else {
                manager.update_storage().await.unwrap();
                storage
                    .ack_event_pair(message.message.nonce_be)
                    .await
                    .unwrap();
                let running = if interruption == "restarted" {
                    let running = Arc::new(JSONStorage::new(&path));
                    TransactionManager::new(running.clone())
                        .load_from_storage()
                        .await
                        .unwrap();
                    running
                } else {
                    storage.clone()
                };
                tokio::fs::remove_file(path.join(&owner_file))
                    .await
                    .unwrap();
                running
            };
            let original_events = tokio::fs::read(path.join("gear_events.json"))
                .await
                .unwrap();
            let original_tx = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
            let original_queued = running_storage.queued_observations().await.unwrap();
            let original_paid = running_storage.paid_observations().await.unwrap();
            let original_pairs = running_storage.pending_event_pairs().await.unwrap();
            assert!(running_storage.record_paid_block(12, H256::repeat_byte(12), &[message.message.nonce_be]).await.is_err(),
                "a missing known owner must HOLD rather than become an unseen paid-first nonce: {interruption}");
            assert!(
                running_storage
                    .ack_event_pair(message.message.nonce_be)
                    .await
                    .is_err(),
                "{interruption}"
            );
            assert_eq!(
                tokio::fs::read(path.join("gear_events.json"))
                    .await
                    .unwrap(),
                original_events
            );
            assert_eq!(
                tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
                original_tx
            );
            assert_eq!(
                running_storage.queued_observations().await.unwrap(),
                original_queued
            );
            assert_eq!(
                running_storage.paid_observations().await.unwrap(),
                original_paid
            );
            assert_eq!(
                running_storage.pending_event_pairs().await.unwrap(),
                original_pairs
            );
            assert!(!path.join(owner_file).exists());
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn paid_lane_rejects_explicit_null_completed_fee_provenance() {
        let path = std::env::temp_dir().join(format!("gear-paid-null-owner-{}", Uuid::new_v4()));
        let (storage, message, _) = paired_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let tx = completed_tx(message.clone()).await;
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        storage
            .ack_event_pair(message.message.nonce_be)
            .await
            .unwrap();
        let owner_path = path.join(JSONStorage::completed_nonce_file(message.message.nonce_be));
        let mut owner: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&owner_path).await.unwrap()).unwrap();
        owner["first_paid"] = serde_json::Value::Null;
        let owner_bytes = serde_json::to_vec(&owner).unwrap();
        tokio::fs::write(&owner_path, &owner_bytes).await.unwrap();
        let original_events = tokio::fs::read(path.join("gear_events.json"))
            .await
            .unwrap();
        let original_tx = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
        let restarted_storage = Arc::new(JSONStorage::new(&path));
        assert!(
            TransactionManager::new(restarted_storage.clone())
                .load_from_storage()
                .await
                .is_err(),
            "explicit null cannot discard the first fee on a paid outbound lane"
        );
        assert!(restarted_storage
            .record_paid_block(12, H256::repeat_byte(12), &[message.message.nonce_be])
            .await
            .is_err());
        assert!(restarted_storage
            .ack_event_pair(message.message.nonce_be)
            .await
            .is_err());
        assert_eq!(tokio::fs::read(owner_path).await.unwrap(), owner_bytes);
        assert_eq!(
            tokio::fs::read(path.join("gear_events.json"))
                .await
                .unwrap(),
            original_events
        );
        assert_eq!(
            tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
            original_tx
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn fee_exempt_source_completion_keeps_optional_first_fee_and_policy_across_restart() {
        for paid_first in [false, true] {
            let path =
                std::env::temp_dir().join(format!("gear-exempt-completion-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            let mut message = msg_in_block(10, 7);
            message.block_hash = H256::repeat_byte(10);
            let mut lane = lane_identity();
            lane.fee_exempt_sources = [[5; 32], message.message.source].into_iter().collect();
            storage.bind_outbound_lane(lane.clone()).await.unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(9), 10)
                .await
                .unwrap();
            storage
                .record_queued_block(10, message.block_hash, &[message.clone()])
                .await
                .unwrap();
            storage
                .record_paid_block(10, message.block_hash, &[])
                .await
                .unwrap();
            let fee_hash = H256::repeat_byte(11);
            let paid_nonces = if paid_first {
                vec![message.message.nonce_be]
            } else {
                vec![]
            };
            storage
                .record_paid_block(11, fee_hash, &paid_nonces)
                .await
                .unwrap();
            let first_paid = paid_first.then_some(PaidObservation {
                nonce: message.message.nonce_be,
                block: 11,
                block_hash: fee_hash,
            });
            let manager = TransactionManager::new(storage.clone());
            let tx = completed_tx(message.clone()).await;
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            let owner_path = path.join(JSONStorage::completed_nonce_file(message.message.nonce_be));
            let original_owner = tokio::fs::read(&owner_path).await.unwrap();
            let original_tx = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
            let owner: CompletedNonce = serde_json::from_slice(&original_owner).unwrap();
            assert_eq!(owner.first_paid, first_paid);
            assert_eq!(owner.lane_identity, lane);

            let restored = Arc::new(JSONStorage::new(&path));
            restored.bind_outbound_lane(lane.clone()).await.unwrap();
            let recovered = TransactionManager::new(restored.clone());
            recovered.load_from_storage().await.unwrap();
            assert_eq!(recovered.completed.read().await[&uuid].message, message);
            // An unpaid exempt source has no fee pair; its original queued observation must still survive.
            assert_eq!(
                restored.queued_observations().await.unwrap(),
                vec![message.clone()]
            );
            // Acknowledge only after the caller's canonical original-receipt check.
            restored
                .ack_event_pair(message.message.nonce_be)
                .await
                .unwrap();
            assert!(restored.queued_observations().await.unwrap().is_empty());
            assert!(restored.paid_observations().await.unwrap().is_empty());
            assert!(restored.pending_event_pairs().await.unwrap().is_empty());
            let unseen = U256::from(999u64).to_big_endian();
            assert_eq!(
                restored
                    .record_paid_block(
                        12,
                        H256::repeat_byte(12),
                        &[message.message.nonce_be, unseen]
                    )
                    .await
                    .unwrap(),
                vec![unseen]
            );
            assert_eq!(
                restored.paid_observations().await.unwrap(),
                vec![PaidObservation {
                    nonce: unseen,
                    block: 12,
                    block_hash: H256::repeat_byte(12)
                }]
            );
            assert!(restored.pending_event_pairs().await.unwrap().is_empty());
            recovered.update_storage().await.unwrap();
            let restarted_storage = Arc::new(JSONStorage::new(&path));
            restarted_storage.bind_outbound_lane(lane).await.unwrap();
            let restarted = TransactionManager::new(restarted_storage);
            restarted.load_from_storage().await.unwrap();
            assert_eq!(
                restarted
                    .completed
                    .read()
                    .await
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![uuid]
            );
            assert_eq!(tokio::fs::read(owner_path).await.unwrap(), original_owner);
            assert_eq!(
                tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
                original_tx
            );
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn outbound_fee_exemption_policy_changes_and_unknown_history_hold() {
        for corruption in [
            "added_source",
            "removed_source",
            "missing_lane_policy",
            "missing_owner_policy",
            "legacy_schema",
        ] {
            let path =
                std::env::temp_dir().join(format!("gear-exemption-policy-hold-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            let mut message = msg_in_block(10, 7);
            message.block_hash = H256::repeat_byte(10);
            let mut lane = lane_identity();
            lane.fee_exempt_sources.insert(message.message.source);
            storage.bind_outbound_lane(lane.clone()).await.unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(9), 10)
                .await
                .unwrap();
            storage
                .record_queued_block(10, message.block_hash, &[message.clone()])
                .await
                .unwrap();
            storage
                .record_paid_block(10, message.block_hash, &[])
                .await
                .unwrap();
            let manager = TransactionManager::new(storage.clone());
            let tx = completed_tx(message.clone()).await;
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            storage
                .ack_event_pair(message.message.nonce_be)
                .await
                .unwrap();
            let events_path = path.join("gear_events.json");
            let owner_path = path.join(JSONStorage::completed_nonce_file(message.message.nonce_be));
            if corruption == "missing_owner_policy" {
                let mut owner: serde_json::Value =
                    serde_json::from_slice(&tokio::fs::read(&owner_path).await.unwrap()).unwrap();
                owner["lane_identity"]
                    .as_object_mut()
                    .unwrap()
                    .remove("fee_exempt_sources");
                tokio::fs::write(&owner_path, serde_json::to_vec(&owner).unwrap())
                    .await
                    .unwrap();
            } else if corruption == "missing_lane_policy" || corruption == "legacy_schema" {
                let mut events: serde_json::Value =
                    serde_json::from_slice(&tokio::fs::read(&events_path).await.unwrap()).unwrap();
                if corruption == "missing_lane_policy" {
                    events["lane_identity"]
                        .as_object_mut()
                        .unwrap()
                        .remove("fee_exempt_sources");
                } else {
                    events["version"] = serde_json::json!(1);
                }
                tokio::fs::write(&events_path, serde_json::to_vec(&events).unwrap())
                    .await
                    .unwrap();
            }
            let original_events = tokio::fs::read(&events_path).await.unwrap();
            let original_owner = tokio::fs::read(&owner_path).await.unwrap();
            let original_tx = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
            let restored = Arc::new(JSONStorage::new(&path));
            if corruption == "added_source" || corruption == "removed_source" {
                let mut changed_lane = lane;
                if corruption == "added_source" {
                    changed_lane.fee_exempt_sources.insert([99; 32]);
                } else {
                    changed_lane.fee_exempt_sources.clear();
                }
                assert!(
                    restored.bind_outbound_lane(changed_lane).await.is_err(),
                    "{corruption}"
                );
            } else {
                assert!(
                    TransactionManager::new(restored.clone())
                        .load_from_storage()
                        .await
                        .is_err(),
                    "{corruption}"
                );
                assert!(
                    restored
                        .record_paid_block(11, H256::repeat_byte(11), &[message.message.nonce_be])
                        .await
                        .is_err(),
                    "{corruption}"
                );
                assert!(
                    restored
                        .ack_event_pair(message.message.nonce_be)
                        .await
                        .is_err(),
                    "{corruption}"
                );
            }
            assert_eq!(tokio::fs::read(events_path).await.unwrap(), original_events);
            assert_eq!(tokio::fs::read(owner_path).await.unwrap(), original_owner);
            assert_eq!(
                tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
                original_tx
            );
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn non_exempt_paid_lane_cannot_persist_handoff_or_completion_without_first_fee() {
        for status in [TxStatus::WaitForMerkleRoot, TxStatus::Completed] {
            let path = std::env::temp_dir()
                .join(format!("gear-non-exempt-missing-fee-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            let mut message = msg_in_block(10, 7);
            message.block_hash = H256::repeat_byte(10);
            storage.bind_outbound_lane(lane_identity()).await.unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(9), 10)
                .await
                .unwrap();
            storage
                .record_queued_block(10, message.block_hash, &[message.clone()])
                .await
                .unwrap();
            storage
                .record_paid_block(10, message.block_hash, &[])
                .await
                .unwrap();
            let original_events = tokio::fs::read(path.join("gear_events.json"))
                .await
                .unwrap();
            let manager = TransactionManager::new(storage.clone());
            let tx = if matches!(status, TxStatus::Completed) {
                completed_tx(message.clone()).await
            } else {
                Transaction::new(message.clone(), status)
            };
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            assert!(manager.update_storage().await.is_err());
            assert!(storage
                .ack_event_pair(message.message.nonce_be)
                .await
                .is_err());
            assert!(!path.join(uuid.to_string()).exists());
            assert!(!path
                .join(JSONStorage::completed_nonce_file(message.message.nonce_be))
                .exists());
            assert_eq!(storage.queued_observations().await.unwrap(), vec![message]);
            assert_eq!(
                tokio::fs::read(path.join("gear_events.json"))
                    .await
                    .unwrap(),
                original_events
            );
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }
    #[tokio::test]
    async fn completed_nonce_preserves_first_fee_and_recovers_crash_before_ack() {
        let path =
            std::env::temp_dir().join(format!("gear-completion-ack-recovery-{}", Uuid::new_v4()));
        let (storage, message, first_paid) = paired_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let tx = completed_tx(message.clone()).await;
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        let owner_path = path.join(JSONStorage::completed_nonce_file(message.message.nonce_be));
        let original = tokio::fs::read(&owner_path).await.unwrap();
        let owner: CompletedNonce = serde_json::from_slice(&original).unwrap();
        assert_eq!(owner.uuid, uuid);
        assert_eq!(owner.message, message);
        assert_eq!(owner.first_paid, Some(first_paid));
        assert_eq!(
            storage.pending_event_pairs().await.unwrap()[0].message,
            message
        );

        // The process stopped after the completed journal save but before acknowledgement.
        let restored = Arc::new(JSONStorage::new(&path));
        let recovered = TransactionManager::new(restored.clone());
        recovered.load_from_storage().await.unwrap();
        assert_eq!(recovered.completed.read().await[&uuid].message, message);
        assert_eq!(
            restored.pending_event_pairs().await.unwrap().len(),
            1,
            "loading saved evidence cannot acknowledge without reauthenticating original finality"
        );
        // Emulate acknowledgement after the caller's canonical original-receipt check.
        restored
            .ack_event_pair(message.message.nonce_be)
            .await
            .unwrap();
        assert!(restored.queued_observations().await.unwrap().is_empty());
        assert!(restored.paid_observations().await.unwrap().is_empty());
        assert!(restored.pending_event_pairs().await.unwrap().is_empty());
        assert!(restored
            .record_paid_block(12, H256::repeat_byte(12), &[message.message.nonce_be])
            .await
            .unwrap()
            .is_empty());
        let mut conflicting_message = message.clone();
        conflicting_message.block = GearBlockNumber(11);
        conflicting_message.block_hash = H256::repeat_byte(11);
        assert!(restored
            .record_queued_block(11, conflicting_message.block_hash, &[conflicting_message])
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            restored
                .event_cursor(GearEventStream::Queued)
                .await
                .unwrap(),
            Some(GearEventCursor {
                block: 10,
                hash: message.block_hash
            })
        );
        recovered.update_storage().await.unwrap();
        let restarted = TransactionManager::new(Arc::new(JSONStorage::new(&path)));
        restarted.load_from_storage().await.unwrap();
        assert_eq!(tokio::fs::read(owner_path).await.unwrap(), original);
        assert_eq!(
            restarted
                .completed
                .read()
                .await
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![uuid]
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn active_held_failed_nonces_retain_first_fee_and_pair_across_restart() {
        for status in [
            TxStatus::WaitForMerkleRoot,
            TxStatus::NeedsReconciliation {
                diagnostic: "unknown inclusion".into(),
            },
            TxStatus::Failed {
                diagnostic: "terminal failure".into(),
            },
        ] {
            let path =
                std::env::temp_dir().join(format!("gear-active-fee-ownership-{}", Uuid::new_v4()));
            let (storage, message, first_paid) = paired_storage(&path).await;
            let manager = TransactionManager::new(storage.clone());
            let tx = Transaction::new(message.clone(), status.clone());
            let uuid = tx.uuid;
            if let TxStatus::Failed { diagnostic } = &status {
                manager.fail_transaction(uuid, diagnostic.clone()).await;
            }
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            assert!(storage
                .outbound_nonce_owned(first_paid.nonce)
                .await
                .unwrap());
            let original = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
            let pair = storage.pending_event_pairs().await.unwrap()[0].clone();
            assert!(storage
                .ack_event_pair(first_paid.nonce)
                .await
                .unwrap_err()
                .to_string()
                .contains("HOLD"));
            let unseen = U256::from(999u64).to_big_endian();
            assert_eq!(
                storage
                    .record_paid_block(12, H256::repeat_byte(12), &[first_paid.nonce, unseen])
                    .await
                    .unwrap(),
                vec![unseen]
            );
            assert_eq!(
                storage.paid_observations().await.unwrap(),
                vec![
                    first_paid.clone(),
                    PaidObservation {
                        nonce: unseen,
                        block: 12,
                        block_hash: H256::repeat_byte(12)
                    }
                ]
            );
            assert_eq!(
                storage.pending_event_pairs().await.unwrap(),
                vec![pair.clone()]
            );
            let restored_storage = Arc::new(JSONStorage::new(&path));
            let recovered = TransactionManager::new(restored_storage.clone());
            let result = recovered.load_from_storage().await;
            match status {
                TxStatus::WaitForMerkleRoot => result.unwrap(),
                _ => assert!(result.unwrap_err().to_string().contains("HOLD")),
            }
            assert_eq!(recovered.transactions.read().await[&uuid].message, message);
            assert!(restored_storage
                .outbound_nonce_owned(first_paid.nonce)
                .await
                .unwrap());
            assert_eq!(
                restored_storage.pending_event_pairs().await.unwrap(),
                vec![pair]
            );
            assert_eq!(
                tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
                original
            );
            assert!(!path
                .join(JSONStorage::completed_nonce_file(first_paid.nonce))
                .exists());

            let events_path = path.join("gear_events.json");
            let mut events: GearEventState =
                serde_json::from_slice(&tokio::fs::read(&events_path).await.unwrap()).unwrap();
            events.pending_pairs.clear();
            let missing_pair = serde_json::to_vec(&events).unwrap();
            tokio::fs::write(&events_path, &missing_pair).await.unwrap();
            assert!(TransactionManager::new(Arc::new(JSONStorage::new(&path)))
                .load_from_storage()
                .await
                .unwrap_err()
                .to_string()
                .contains("HOLD"));
            assert_eq!(tokio::fs::read(events_path).await.unwrap(), missing_pair);
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn conflicting_or_missing_completed_ownership_holds_original_discovery() {
        for corruption in [
            "source",
            "lane",
            "message",
            "uuid",
            "fee_nonce",
            "missing_fee",
            "missing_uuid",
            "missing_owner",
            "retained_pair_missing",
            "version",
        ] {
            let path =
                std::env::temp_dir().join(format!("gear-completed-owner-hold-{}", Uuid::new_v4()));
            let (storage, message, _) = paired_storage(&path).await;
            let manager = TransactionManager::new(storage.clone());
            let tx = completed_tx(message.clone()).await;
            let uuid_path = path.join(tx.uuid.to_string());
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            if corruption != "retained_pair_missing" {
                storage
                    .ack_event_pair(message.message.nonce_be)
                    .await
                    .unwrap();
            }
            let owner_path = path.join(JSONStorage::completed_nonce_file(message.message.nonce_be));
            let mut owner: serde_json::Value =
                serde_json::from_slice(&tokio::fs::read(&owner_path).await.unwrap()).unwrap();
            match corruption {
                "source" => {
                    owner["genesis_hash"] = serde_json::to_value(H256::repeat_byte(99)).unwrap()
                }
                "lane" => {
                    owner["lane_identity"]["message_queue_address"] =
                        serde_json::to_value(H160::repeat_byte(99)).unwrap()
                }
                "message" => owner["message"]["message"]["payload"] = serde_json::json!([99]),
                "uuid" => owner["uuid"] = serde_json::to_value(Uuid::new_v4()).unwrap(),
                "fee_nonce" => {
                    owner["first_paid"]["nonce"] = serde_json::to_value([99u8; 32]).unwrap()
                }
                "missing_fee" => {
                    owner.as_object_mut().unwrap().remove("first_paid");
                }
                "version" => owner["version"] = serde_json::json!(2),
                "missing_uuid" | "missing_owner" | "retained_pair_missing" => {}
                _ => unreachable!(),
            }
            if corruption == "missing_owner" {
                tokio::fs::remove_file(&owner_path).await.unwrap();
            } else {
                tokio::fs::write(&owner_path, serde_json::to_vec(&owner).unwrap())
                    .await
                    .unwrap();
            }
            if corruption == "missing_uuid" {
                tokio::fs::remove_file(&uuid_path).await.unwrap();
            }
            let event_path = path.join("gear_events.json");
            if corruption == "retained_pair_missing" {
                let mut events: GearEventState =
                    serde_json::from_slice(&tokio::fs::read(&event_path).await.unwrap()).unwrap();
                events.pending_pairs.clear();
                tokio::fs::write(&event_path, serde_json::to_vec(&events).unwrap())
                    .await
                    .unwrap();
            }
            let original_events = tokio::fs::read(&event_path).await.unwrap();
            let restored = Arc::new(JSONStorage::new(&path));
            assert!(
                TransactionManager::new(restored.clone())
                    .load_from_storage()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("HOLD"),
                "{corruption}"
            );
            assert!(
                restored
                    .record_paid_block(12, H256::repeat_byte(12), &[message.message.nonce_be])
                    .await
                    .is_err(),
                "{corruption}"
            );
            assert!(
                restored
                    .ack_event_pair(message.message.nonce_be)
                    .await
                    .is_err(),
                "{corruption}"
            );
            assert_eq!(tokio::fs::read(event_path).await.unwrap(), original_events);
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn legacy_transaction_migration_retains_original_evidence_and_holds_ambiguous_sends() {
        use crate::message_relayer::gear_to_eth::tx_manager::TxStatus;
        let path = std::env::temp_dir().join(format!("gear-outbound-migrate-{}", Uuid::new_v4()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_outbound_lane(manual_lane_identity())
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 42)
            .await
            .unwrap();
        let mut tx = Transaction::new(msg_in_block(42, 7), TxStatus::WaitForMerkleRoot);
        let uuid = tx.uuid;
        let journal = path.join(uuid.to_string());
        let mut original = serde_json::to_value(&tx).unwrap();
        let object = original.as_object_mut().unwrap();
        object.remove("journal_version");
        object.insert("dropped_retries".into(), serde_json::json!(2));
        let bytes = serde_json::to_vec(&original).unwrap();
        tokio::fs::write(&journal, &bytes).await.unwrap();
        let restored = TransactionManager::new(storage.clone());
        restored.load_from_storage().await.unwrap();
        assert_eq!(
            restored.transactions.read().await[&uuid]
                .legacy_evidence
                .as_ref(),
            Some(&original)
        );
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        restored.update_storage().await.unwrap();
        let upgraded: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&journal).await.unwrap()).unwrap();
        assert_eq!(upgraded["journal_version"], 2);
        assert_eq!(upgraded["legacy_evidence"], original);
        for corrupt in [
            serde_json::json!({"journal_version": 4}),
            serde_json::json!({"unexpected_field": 1}),
        ] {
            let mut invalid = upgraded.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .extend(corrupt.as_object().unwrap().clone());
            let bytes = serde_json::to_vec(&invalid).unwrap();
            tokio::fs::write(&journal, &bytes).await.unwrap();
            assert!(TransactionManager::new(storage.clone())
                .load_from_storage()
                .await
                .is_err());
            assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        }
        let mut unsigned_with_attempt = original.clone();
        unsigned_with_attempt["status"] = serde_json::json!("WaitForMerkleRoot");
        unsigned_with_attempt["ethereum_tx_attempts"] =
            serde_json::json!([ethereum_client::TxHash::from([9; 32])]);
        let bytes = serde_json::to_vec(&unsigned_with_attempt).unwrap();
        tokio::fs::write(&journal, &bytes).await.unwrap();
        assert!(TransactionManager::new(storage.clone())
            .load_from_storage()
            .await
            .is_err());
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        tokio::fs::remove_file(path.join("transaction_status.json"))
            .await
            .unwrap();
        let root = crate::message_relayer::common::RelayedMerkleRoot {
            block: GearBlockNumber(43),
            block_hash: H256::repeat_byte(8),
            timestamp: 123,
            authority_set_id: AuthoritySetId(1),
            merkle_root: H256(tx.message_hash),
        };
        tx.status = TxStatus::SendMessage(
            root,
            gear_rpc_client::dto::MerkleProof {
                root: tx.message_hash,
                proof: vec![],
                num_leaves: 1,
                leaf_index: 0,
            },
        );
        let mut ambiguous = serde_json::to_value(tx).unwrap();
        ambiguous.as_object_mut().unwrap().remove("journal_version");
        let bytes = serde_json::to_vec(&ambiguous).unwrap();
        tokio::fs::write(&journal, &bytes).await.unwrap();
        let held = TransactionManager::new(storage);
        assert!(held
            .load_from_storage()
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            held.transactions.read().await[&uuid]
                .legacy_evidence
                .as_ref(),
            Some(&ambiguous)
        );
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn unprocessed_blocks_cleared_after_all_messages_dequeued() {
        let storage = BlockStorage::new();
        let block = GearBlockNumber(10);
        let block_hash = H256::from_low_u64_be(999);

        let n1 = U256::from(1u64).to_big_endian();
        let n2 = U256::from(2u64).to_big_endian();

        storage
            .add_block(block, block_hash, [n1, n2].into_iter())
            .await;

        let unprocessed = storage.unprocessed_blocks().await;
        assert_eq!(unprocessed.blocks, vec![(block_hash, 10)]);
        assert_eq!(unprocessed.first_block, Some((block_hash, 10)));
        assert_eq!(unprocessed.last_block, Some((block_hash, 10)));

        storage.complete_transaction(&msg_in_block(10, 1)).await;
        let unprocessed = storage.unprocessed_blocks().await;
        assert_eq!(unprocessed.blocks, vec![(block_hash, 10)]);
        assert_eq!(unprocessed.first_block, Some((block_hash, 10)));
        assert_eq!(unprocessed.last_block, Some((block_hash, 10)));

        storage.complete_transaction(&msg_in_block(10, 2)).await;
        let unprocessed = storage.unprocessed_blocks().await;
        assert!(unprocessed.blocks.is_empty());
        assert_eq!(unprocessed.first_block, None);
        assert_eq!(unprocessed.last_block, None);
    }

    #[tokio::test]
    async fn add_block_does_not_reintroduce_completed_messages() {
        let storage = BlockStorage::new();
        let block = GearBlockNumber(7);
        let block_hash = H256::from_low_u64_be(777);

        let n1 = U256::from(1u64).to_big_endian();
        let n2 = U256::from(2u64).to_big_endian();

        storage
            .add_block(block, block_hash, [n1, n2].into_iter())
            .await;
        storage.complete_transaction(&msg_in_block(7, 1)).await;

        // Replay of the same block should be ignored and not restore nonce=1.
        storage
            .add_block(block, block_hash, [n1, n2].into_iter())
            .await;

        assert!(!storage.is_message_pending(block, n1).await);
        assert!(storage.is_message_pending(block, n2).await);
    }

    #[tokio::test]
    async fn legacy_failed_uuid_with_no_terminal_message_holds() {
        use crate::message_relayer::gear_to_eth::tx_manager::TxStatus;
        use std::sync::Arc;

        let path = std::env::temp_dir().join(format!(
            "gear-bridge-relayer-legacy-failed-test-{}",
            Uuid::new_v4()
        ));
        tokio::fs::create_dir_all(&path).await.unwrap();

        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_outbound_lane(manual_lane_identity())
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 43)
            .await
            .unwrap();
        let tx = Transaction::new(msg_in_block(43, 8), TxStatus::WaitForMerkleRoot);
        let uuid = tx.uuid;
        storage.write_tx(&uuid, &tx).await.unwrap();
        tokio::fs::write(
            path.join("failed"),
            serde_json::to_vec(&BTreeMap::from([(uuid, "legacy failure")])).unwrap(),
        )
        .await
        .unwrap();

        let restored = TransactionManager::new(storage.clone());
        let error = storage.load(&restored).await.unwrap_err().to_string();
        assert!(error.contains("HOLD"));
        assert!(error.contains(&uuid.to_string()));
        assert_eq!(
            restored.failed.read().await.get(&uuid).map(String::as_str),
            Some("legacy failure")
        );

        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn paired_events_and_cursors_survive_restart_until_acknowledged() {
        let path = std::env::temp_dir().join(format!("gear-event-storage-{}", Uuid::new_v4()));
        let storage = JSONStorage::new(&path);
        let mut lane = lane_identity();
        lane.fee_exempt_sources.insert([3; 32]);
        storage.bind_outbound_lane(lane).await.unwrap();
        let genesis = H256::repeat_byte(0x11);
        storage.bind_event_chain(genesis, 10).await.unwrap();

        let queue_hash = H256::repeat_byte(0x22);
        let mut queued = msg_in_block(10, 7);
        queued.block_hash = queue_hash;
        let nonce = queued.message.nonce_be;
        storage
            .record_queued_block(10, queue_hash, &[queued.clone()])
            .await
            .unwrap();
        assert!(storage.pending_event_pairs().await.unwrap().is_empty());
        assert_eq!(
            storage.queued_observations().await.unwrap(),
            vec![queued.clone()]
        );
        assert_eq!(
            storage
                .replay_from_block(99, GearEventStream::Paid)
                .await
                .unwrap(),
            10
        );
        assert_eq!(
            storage
                .replay_from_block(99, GearEventStream::Queued)
                .await
                .unwrap(),
            11
        );
        let paid_hash = H256::repeat_byte(0x33);
        storage
            .record_paid_block(10, paid_hash, &[nonce])
            .await
            .unwrap();
        assert_eq!(
            storage
                .replay_from_block(99, GearEventStream::Paid)
                .await
                .unwrap(),
            11
        );
        assert_eq!(storage.pending_event_pairs().await.unwrap().len(), 1);
        assert_eq!(
            storage.event_cursor(GearEventStream::Queued).await.unwrap(),
            Some(GearEventCursor {
                block: 10,
                hash: queue_hash
            })
        );
        assert_eq!(
            storage.event_cursor(GearEventStream::Paid).await.unwrap(),
            Some(GearEventCursor {
                block: 10,
                hash: paid_hash
            })
        );
        assert!(storage
            .record_queued_block(12, H256::repeat_byte(0x66), &[])
            .await
            .is_err());
        assert!(storage
            .record_queued_block(10, H256::repeat_byte(0x66), &[])
            .await
            .is_err());
        assert_eq!(
            storage
                .event_cursor(GearEventStream::Queued)
                .await
                .unwrap()
                .unwrap()
                .block,
            10
        );
        drop(storage);

        let root_storage = JSONStorage::new(&path);
        root_storage.save_eth_cursor(123).await.unwrap();
        let restored = Arc::new(JSONStorage::new(&path));
        assert_eq!(restored.load_eth_cursor().await.unwrap(), Some(123));
        assert_eq!(restored.event_start_block().await.unwrap(), Some(10));
        assert_eq!(
            restored
                .replay_from_block(99, GearEventStream::Queued)
                .await
                .unwrap(),
            11
        );
        assert_eq!(
            restored.pending_event_pairs().await.unwrap()[0].message,
            queued.clone()
        );
        assert!(restored.bind_event_chain(genesis, 10).await.is_ok());
        assert!(restored
            .bind_event_chain(H256::repeat_byte(0x44), 10)
            .await
            .is_err());

        let tx_manager = TransactionManager::new(restored.clone());
        restored.load(&tx_manager).await.unwrap();

        tx_manager
            .add_transaction(completed_tx(queued.clone()).await)
            .await;
        tx_manager.update_storage().await.unwrap();
        restored.ack_event_pair(nonce).await.unwrap();
        restored
            .record_queued_block(10, queue_hash, &[queued])
            .await
            .unwrap();
        restored
            .record_paid_block(10, paid_hash, &[nonce])
            .await
            .unwrap();
        assert!(restored.queued_observations().await.unwrap().is_empty());
        assert!(restored.paid_observations().await.unwrap().is_empty());
        assert!(restored.pending_event_pairs().await.unwrap().is_empty());

        let mut exempt = msg_in_block(11, 8);
        exempt.message.source = [3; 32];
        let exempt_hash = H256::repeat_byte(0x55);
        exempt.block_hash = exempt_hash;
        let exempt_nonce = exempt.message.nonce_be;
        restored
            .record_queued_block(11, exempt_hash, &[exempt.clone()])
            .await
            .unwrap();
        assert_eq!(
            restored
                .replay_from_block(99, GearEventStream::Queued)
                .await
                .unwrap(),
            12
        );
        assert_eq!(
            restored
                .replay_from_block(99, GearEventStream::Paid)
                .await
                .unwrap(),
            11
        );
        tx_manager.add_transaction(completed_tx(exempt).await).await;
        tx_manager.update_storage().await.unwrap();
        restored.ack_event_pair(exempt_nonce).await.unwrap();
        assert!(restored.queued_observations().await.unwrap().is_empty());
        assert!(restored
            .record_paid_block(11, exempt_hash, &[exempt_nonce])
            .await
            .unwrap()
            .is_empty());
        assert!(restored.paid_observations().await.unwrap().is_empty());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn event_journal_rejects_unknown_schema_versions() {
        let path = std::env::temp_dir().join(format!("gear-event-schema-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&path).await.unwrap();
        let state = GearEventState {
            version: 3,
            ..Default::default()
        };
        tokio::fs::write(
            path.join("gear_events.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .await
        .unwrap();

        let storage = JSONStorage::new(&path);
        assert!(storage.event_start_block().await.is_err());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn event_journal_rejects_existing_storage_without_event_journal() {
        let path = std::env::temp_dir().join(format!("gear-event-legacy-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(path.join("legacy-transaction"), b"{}")
            .await
            .unwrap();

        let storage = JSONStorage::new(&path);
        assert!(storage
            .bind_event_chain(H256::repeat_byte(0x11), 10)
            .await
            .is_err());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_transaction_and_block_saves_preserve_a_complete_snapshot() {
        use crate::message_relayer::gear_to_eth::tx_manager::TxStatus;

        let path = std::env::temp_dir().join(format!("gear-outbound-saves-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&path).await.unwrap();
        let storage = Arc::new(JSONStorage::new(&path));
        storage.bind_outbound_lane(lane_identity()).await.unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 1)
            .await
            .unwrap();
        let message = msg_in_block(1, 1);
        storage
            .record_queued_block(1, message.block_hash, &[message])
            .await
            .unwrap();
        storage
            .record_paid_block(
                1,
                msg_in_block(1, 1).block_hash,
                &[U256::from(1u64).to_big_endian()],
            )
            .await
            .unwrap();
        let manager = Arc::new(TransactionManager::new(storage.clone()));
        manager
            .add_transaction(Transaction::new(
                msg_in_block(1, 1),
                TxStatus::WaitForMerkleRoot,
            ))
            .await;
        for block in 1..=32 {
            storage
                .block_storage()
                .add_block(
                    GearBlockNumber(block),
                    H256::from_low_u64_be(block as u64),
                    [U256::from(block).to_big_endian()].into_iter(),
                )
                .await;
        }
        let start = Arc::new(tokio::sync::Barrier::new(9));
        let mut tasks = Vec::new();
        for index in 0..8 {
            let storage = storage.clone();
            let manager = manager.clone();
            let start = start.clone();
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                if index == 0 {
                    storage.save(&manager).await
                } else {
                    storage.save_blocks().await
                }
            }));
        }
        start.wait().await;
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert!(!path.join("blocks.json.new").exists());
        let restored = JSONStorage::new(&path);
        let restored_manager = TransactionManager::new(Arc::new(NoStorage::new()));
        restored.load(&restored_manager).await.unwrap();
        assert_eq!(
            restored
                .block_storage()
                .unprocessed_blocks()
                .await
                .blocks
                .len(),
            32
        );
        assert_eq!(restored_manager.transactions.read().await.len(), 1);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_and_malformed_outbound_journals_hold_original_transactions() {
        use crate::message_relayer::gear_to_eth::tx_manager::TxStatus;

        let path = std::env::temp_dir().join(format!("gear-outbound-hold-{}", Uuid::new_v4()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_outbound_lane(manual_lane_identity())
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 42)
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let tx = Transaction::new(msg_in_block(42, 7), TxStatus::WaitForMerkleRoot);
        let committed_path = path.join(tx.uuid.to_string());
        manager.add_transaction(tx).await;
        storage.save(&manager).await.unwrap();
        let committed = tokio::fs::read(&committed_path).await.unwrap();
        let temporary = path.join(".state").join(format!(
            "{}.new",
            committed_path.file_name().unwrap().to_string_lossy()
        ));
        tokio::fs::write(&temporary, b"{truncated").await.unwrap();
        let restored = TransactionManager::new(storage.clone());
        assert!(storage
            .load(&restored)
            .await
            .unwrap_err()
            .to_string()
            .contains(".new"));
        assert!(manager.update_storage().await.is_err());
        assert!(path.join(".state/save.pending").exists());
        assert_eq!(tokio::fs::read(&committed_path).await.unwrap(), committed);
        assert!(storage
            .bind_event_chain(H256::repeat_byte(0x11), 42)
            .await
            .is_err());

        tokio::fs::remove_file(temporary).await.unwrap();
        tokio::fs::remove_file(path.join(".state/save.pending"))
            .await
            .unwrap();
        let block_temporary = path.join("blocks.json.new");
        tokio::fs::write(&block_temporary, b"{truncated")
            .await
            .unwrap();
        assert!(storage
            .load(&restored)
            .await
            .unwrap_err()
            .to_string()
            .contains("blocks.json.new"));
        tokio::fs::remove_file(block_temporary).await.unwrap();
        tokio::fs::write(path.join("failed"), b"{truncated")
            .await
            .unwrap();
        let error = storage.load(&restored).await.unwrap_err();
        assert!(error.to_string().contains("Failed to parse 'failed'"));
        assert_eq!(tokio::fs::read(committed_path).await.unwrap(), committed);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn outbound_lane_cannot_rebind_queue_or_unbound_root_cursor() {
        let path = std::env::temp_dir().join(format!("gear-outbound-lane-{}", Uuid::new_v4()));
        let storage = JSONStorage::new(&path);
        storage.bind_outbound_lane(lane_identity()).await.unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(0x11), 10)
            .await
            .unwrap();
        storage.save_eth_cursor(123).await.unwrap();

        let mut other_queue = lane_identity();
        other_queue.message_queue_address = H160::repeat_byte(0x55);
        let restored = JSONStorage::new(&path);
        assert!(restored
            .bind_outbound_lane(other_queue)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(restored.load_eth_cursor().await.unwrap(), Some(123));
        tokio::fs::remove_dir_all(path).await.unwrap();

        let legacy = std::env::temp_dir().join(format!("gear-unbound-cursor-{}", Uuid::new_v4()));
        let unbound = JSONStorage::new(&legacy);
        unbound
            .write_atomic_json("gear_events.json", &GearEventState::default())
            .await
            .unwrap();
        unbound.save_eth_cursor(123).await.unwrap();
        let error = unbound
            .bind_outbound_lane(lane_identity())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HOLD") && error.contains("root cursor"));
        assert_eq!(unbound.load_eth_cursor().await.unwrap(), Some(123));
        tokio::fs::remove_dir_all(legacy).await.unwrap();
    }
}
