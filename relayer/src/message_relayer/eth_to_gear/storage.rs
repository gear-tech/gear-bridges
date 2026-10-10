use super::tx_manager::{Transaction, TransactionManager, TxStatus};
use crate::message_relayer::common::{EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot};
use anyhow::Context;
use async_trait::async_trait;
use ethereum_client::TxHash;
use primitive_types::{H160, H256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock},
};
use uuid::Uuid;

/// Storage type implementing
/// storage for Ethereum blocks.
pub struct BlockStorage {
    blocks: RwLock<BTreeMap<EthereumSlotNumber, Block>>,
    save_lock: Mutex<()>,
    n_to_keep: usize,
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    pub number: EthereumBlockNumber,
    #[serde(default)]
    pub hash: Option<H256>,
    pub transactions: HashSet<TxHash>,
}

impl Block {
    pub fn is_processed(&self) -> bool {
        self.transactions.is_empty()
    }
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

    pub fn blocks_raw(&self) -> &RwLock<BTreeMap<EthereumSlotNumber, Block>> {
        &self.blocks
    }

    pub async fn complete_transaction(&self, tx: &TxHashWithSlot) {
        let mut blocks = self.blocks.write().await;
        let Some(block) = blocks.get_mut(&tx.slot_number) else {
            log::warn!(
                "Block at slot #{} associated with transaction #{:?} not found in storage",
                tx.slot_number,
                tx.tx_hash
            );
            return;
        };

        if !block.transactions.remove(&tx.tx_hash) {
            log::warn!(
                "Transaction #{:?} in block at slot #{} is already completed",
                tx.slot_number.0,
                tx.tx_hash
            );
        };
    }

    pub async fn is_transaction_pending(&self, slot: EthereumSlotNumber, tx_hash: TxHash) -> bool {
        let blocks = self.blocks.read().await;
        let Some(block) = blocks.get(&slot) else {
            return false;
        };

        block.transactions.contains(&tx_hash)
    }

    pub async fn add_block(
        &self,
        slot: EthereumSlotNumber,
        number: EthereumBlockNumber,
        hash: H256,
        txs: impl Iterator<Item = TxHash>,
    ) -> anyhow::Result<()> {
        let mut blocks = self.blocks.write().await;
        if let Some(existing) = blocks.get(&slot) {
            anyhow::ensure!(
                existing.number == number && existing.hash == Some(hash),
                "HOLD: finalized inbound block identity changed at slot {}",
                slot.0
            );
            return Ok(());
        }
        blocks.insert(
            slot,
            Block {
                number,
                hash: Some(hash),
                transactions: txs.collect(),
            },
        );
        Ok(())
    }

    pub async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        let blocks = self.blocks.read().await;

        let unprocessed = blocks
            .iter()
            .filter_map(|(_, block)| (!block.is_processed()).then_some(block.number))
            .collect::<Vec<_>>();

        let last_block = blocks.last_key_value().map(|(_, block)| block.number);

        UnprocessedBlocks {
            unprocessed,
            last_block,
        }
    }

    pub async fn prune(&self) {
        let mut blocks = self.blocks.write().await;

        let mut remove_until = None;

        for (index, (slot, block)) in blocks.iter().enumerate() {
            if index + self.n_to_keep > blocks.len() {
                remove_until = Some(*slot);
                break;
            }

            if !block.is_processed() {
                remove_until = Some(*slot);
                break;
            }
        }

        if let Some(remove_until) = remove_until {
            *blocks = blocks.split_off(&remove_until);
        }
    }

    pub async fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let _save_guard = self.save_lock.lock().await;
        let path = path.as_ref();
        let blocks_new = path.join("blocks.json.new");
        let blocks_old = path.join("blocks.json");
        let mut blocks_file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&blocks_new)
            .await
            .with_context(|| {
                format!(
                    "Failed to open or create blocks file in storage path: '{}'",
                    path.display()
                )
            })?;
        // just keep 100 processed blocks in JSON storage for now...
        self.prune().await;
        let blocks = self.blocks.read().await;
        let blocks_json = serde_json::to_string::<BTreeMap<EthereumSlotNumber, Block>>(&*blocks)?;
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
        blocks_file.sync_all().await.with_context(|| {
            format!(
                "Failed to sync blocks file in storage path: '{}'",
                path.display()
            )
        })?;
        drop(blocks_file);

        tokio::fs::rename(blocks_new, blocks_old)
            .await
            .with_context(|| {
                format!(
                    "Failed to rename new blocks file in storage path: '{}'",
                    path.display()
                )
            })?;
        tokio::fs::File::open(path)
            .await
            .with_context(|| {
                format!(
                    "Failed to open block storage directory for syncing: '{}'",
                    path.display()
                )
            })?
            .sync_all()
            .await
            .with_context(|| {
                format!(
                    "Failed to sync block storage directory: '{}'",
                    path.display()
                )
            })?;
        Ok(())
    }
}

pub struct UnprocessedBlocks {
    pub last_block: Option<EthereumBlockNumber>,
    pub unprocessed: Vec<EthereumBlockNumber>,
}

#[async_trait]
pub trait Storage: Send + Sync {
    fn block_storage(&self) -> &BlockStorage;
    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn save_blocks(&self) -> anyhow::Result<()>;
}

#[cfg(test)]
pub(crate) struct NoStorage(BlockStorage);

#[cfg(test)]
impl Default for NoStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl NoStorage {
    pub fn new() -> Self {
        Self(BlockStorage::new())
    }
}

#[cfg(test)]
#[async_trait]
impl Storage for NoStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.0
    }
    async fn save(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }

    async fn load(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }
}

/// Simple storage for transactions which keeps them in a JSON file under
/// specified directory.
pub struct JSONStorage {
    path: PathBuf,
    block_storage: BlockStorage,
    runtime_identity: RwLock<Option<InboundRuntimeIdentity>>,
    manual_identity: RwLock<Option<ManualInboundIdentity>>,
    save_lock: Mutex<()>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundRuntimeIdentity {
    pub ethereum_chain_id: u64,
    pub ethereum_genesis_hash: H256,
    pub ethereum_start_block: u64,
    pub erc20_manager_address: Option<H160>,
    pub bridging_payment_address: Option<H160>,
    pub gear_genesis_hash: H256,
    pub vft_manager_address: H256,
    pub checkpoint_light_client_address: H256,
    pub historical_proxy_address: H256,
    pub gear_sender: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ManualInboundIdentity {
    pub tx_hash: TxHash,
    pub slot_number: EthereumSlotNumber,
    pub ethereum_block_hash: H256,
    pub transaction_index: u64,
    pub receiver_route: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct StoredState {
    schema_version: u32,
    #[serde(default)]
    runtime_identity: Option<InboundRuntimeIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manual_identity: Option<ManualInboundIdentity>,
    transactions: BTreeMap<Uuid, Transaction>,
    completed: BTreeMap<Uuid, Transaction>,
    failed: BTreeMap<Uuid, String>,
}

const STATE_FILE: &str = "state.json";
// Schema5 has original-proof identity; schema6 adds its separately signed native reconciliation.
const STATE_SCHEMA_VERSION: u32 = 6;

impl JSONStorage {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            block_storage: BlockStorage::new(),
            runtime_identity: RwLock::new(None),
            manual_identity: RwLock::new(None),
            save_lock: Mutex::new(()),
        }
    }

    pub async fn bind_runtime_identity(
        &self,
        identity: InboundRuntimeIdentity,
    ) -> anyhow::Result<()> {
        self.bind_identity(identity, None).await
    }

    pub(super) async fn bind_manual_identity(
        &self,
        identity: InboundRuntimeIdentity,
        manual: ManualInboundIdentity,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            manual.tx_hash != TxHash::ZERO
                && manual.ethereum_block_hash != H256::zero()
                && !manual.receiver_route.is_empty(),
            "HOLD: incomplete manual inbound target/route identity"
        );
        self.bind_identity(identity, Some(manual)).await
    }

    async fn bind_identity(
        &self,
        identity: InboundRuntimeIdentity,
        manual: Option<ManualInboundIdentity>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            identity.ethereum_chain_id != 0
                && identity.ethereum_genesis_hash != H256::zero()
                && identity.gear_genesis_hash != H256::zero()
                && identity.vft_manager_address != H256::zero()
                && identity.checkpoint_light_client_address != H256::zero()
                && identity.historical_proxy_address != H256::zero()
                && identity.gear_sender != [0; 32]
                && (if manual.is_some() {
                    identity.erc20_manager_address.is_none()
                        && identity.bridging_payment_address.is_none()
                } else {
                    identity.erc20_manager_address.is_some()
                        ^ identity.bridging_payment_address.is_some()
                })
                && identity.erc20_manager_address != Some(H160::zero())
                && identity.bridging_payment_address != Some(H160::zero()),
            "HOLD: incomplete or conflicting inbound Ethereum/Gear runtime identity"
        );
        let _guard = self.save_lock.lock().await;
        tokio::fs::create_dir_all(&self.path).await?;
        anyhow::ensure!(
            !self.path.join(format!("{STATE_FILE}.new")).exists()
                && !self.path.join("blocks.json.new").exists(),
            "HOLD: interrupted inbound journal snapshot"
        );
        let existing = self.read_state().await?;
        let blocks_path = self.path.join("blocks.json");
        let blocks: Option<BTreeMap<EthereumSlotNumber, Block>> = if blocks_path.exists() {
            Some(serde_json::from_slice(
                &tokio::fs::read(&blocks_path).await?,
            )?)
        } else {
            None
        };
        anyhow::ensure!(
            existing.is_none() || blocks.is_some(),
            "Incomplete block storage in '{}': state.json exists without blocks.json",
            self.path.display()
        );
        if let Some(mut state) = existing {
            anyhow::ensure!(
                state.manual_identity == manual,
                "HOLD: manual inbound target/route changed or journal belongs to another relay mode"
            );
            if let Some(previous) = &state.runtime_identity {
                anyhow::ensure!(
                    previous == &identity,
                    "HOLD: inbound runtime identity changed"
                );
            } else {
                anyhow::ensure!(
                    manual.is_none()
                        && state.transactions.is_empty()
                        && state.completed.is_empty()
                        && state.failed.is_empty()
                        && blocks.as_ref().is_none_or(BTreeMap::is_empty),
                    "HOLD: inbound journal history lacks an immutable runtime identity"
                );
                state.runtime_identity = Some(identity.clone());
                self.write_state(&state).await?;
            }
        } else {
            self.reject_legacy_journal().await?;
            anyhow::ensure!(
                blocks.is_none(),
                "HOLD: inbound block snapshot survived without transaction state/identity"
            );
            self.block_storage.save(&self.path).await?;
            self.write_state(&StoredState {
                schema_version: STATE_SCHEMA_VERSION,
                runtime_identity: Some(identity.clone()),
                manual_identity: manual.clone(),
                transactions: BTreeMap::new(),
                completed: BTreeMap::new(),
                failed: BTreeMap::new(),
            })
            .await?;
        }
        *self.runtime_identity.write().await = Some(identity);
        *self.manual_identity.write().await = manual;
        Ok(())
    }

    fn preserve_unknown_json(
        current: &mut serde_json::Value,
        known: &serde_json::Value,
        previous: &serde_json::Value,
    ) {
        match (current, known, previous) {
            (
                serde_json::Value::Object(current),
                serde_json::Value::Object(known),
                serde_json::Value::Object(previous),
            ) => {
                for (key, previous) in previous {
                    if let Some(known) = known.get(key) {
                        if let Some(current) = current.get_mut(key) {
                            Self::preserve_unknown_json(current, known, previous);
                        }
                    } else {
                        current
                            .entry(key.clone())
                            .or_insert_with(|| previous.clone());
                    }
                }
            }
            (
                serde_json::Value::Array(current),
                serde_json::Value::Array(known),
                serde_json::Value::Array(previous),
            ) => {
                for ((current, known), previous) in current.iter_mut().zip(known).zip(previous) {
                    Self::preserve_unknown_json(current, known, previous);
                }
            }
            _ => {}
        }
    }

    async fn write_state(&self, state: &StoredState) -> anyhow::Result<()> {
        let state_new = self.path.join(format!("{STATE_FILE}.new"));
        let state_old = self.path.join(STATE_FILE);
        let mut value = serde_json::to_value(state)?;
        if state_old.exists() {
            let previous: serde_json::Value =
                serde_json::from_slice(&tokio::fs::read(&state_old).await?)?;
            let known =
                serde_json::to_value(serde_json::from_value::<StoredState>(previous.clone())?)?;
            Self::preserve_unknown_json(&mut value, &known, &previous);
            // Transaction IDs survive movement between pending/completed maps.
            for section in ["transactions", "completed"] {
                if let Some(rows) = value[section].as_object_mut() {
                    for (id, row) in rows {
                        for old_section in ["transactions", "completed"] {
                            if let (Some(known), Some(previous)) =
                                (known[old_section].get(id), previous[old_section].get(id))
                            {
                                Self::preserve_unknown_json(row, known, previous);
                            }
                        }
                    }
                }
            }
        }
        let json = serde_json::to_string(&value)?;
        let mut file = tokio::fs::File::create(&state_new).await.with_context(|| {
            format!(
                "Failed to create temporary state file in storage path: '{}'",
                self.path.display()
            )
        })?;
        file.write_all(json.as_bytes()).await.with_context(|| {
            format!(
                "Failed to write temporary state file in storage path: '{}'",
                self.path.display()
            )
        })?;
        file.flush().await.with_context(|| {
            format!(
                "Failed to flush temporary state file in storage path: '{}'",
                self.path.display()
            )
        })?;
        file.sync_all().await.with_context(|| {
            format!(
                "Failed to sync temporary state file in storage path: '{}'",
                self.path.display()
            )
        })?;
        drop(file);

        tokio::fs::rename(&state_new, &state_old)
            .await
            .with_context(|| {
                format!(
                    "Failed to atomically replace state file in storage path: '{}'",
                    self.path.display()
                )
            })?;
        tokio::fs::File::open(&self.path)
            .await
            .with_context(|| {
                format!(
                    "Failed to open transaction journal directory for syncing: '{}'",
                    self.path.display()
                )
            })?
            .sync_all()
            .await
            .with_context(|| {
                format!(
                    "Failed to sync transaction journal directory: '{}'",
                    self.path.display()
                )
            })?;
        Ok(())
    }

    async fn read_state(&self) -> anyhow::Result<Option<StoredState>> {
        let state_path = self.path.join(STATE_FILE);
        if !state_path.exists() {
            return Ok(None);
        }
        let contents = tokio::fs::read_to_string(&state_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to read state file in storage path: '{}'",
                    self.path.display()
                )
            })?;
        let value: serde_json::Value =
            serde_json::from_str(&contents).context("Failed to parse transaction journal JSON")?;
        let schema_version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Incompatible transaction journal schema in '{}': missing integer schema_version; expected {STATE_SCHEMA_VERSION}",
                    self.path.display()
                )
            })?;
        anyhow::ensure!(
            matches!(schema_version, 5 | 6),
            "Incompatible transaction journal schema in '{}': found {schema_version}, expected {STATE_SCHEMA_VERSION}",
            self.path.display()
        );
        let mut state: StoredState =
            serde_json::from_value(value).context("Failed to deserialize transaction journal")?;
        state.schema_version = STATE_SCHEMA_VERSION;
        Ok(Some(state))
    }

    async fn load_state(&self, tx_manager: &TransactionManager) -> anyhow::Result<bool> {
        let Some(state) = self.read_state().await? else {
            return Ok(false);
        };
        let expected = self.runtime_identity.read().await;
        anyhow::ensure!(
            expected.is_some() && state.runtime_identity.as_ref() == expected.as_ref(),
            "HOLD: inbound runtime identity missing or changed before journal replay"
        );
        drop(expected);
        anyhow::ensure!(
            state.manual_identity.as_ref() == self.manual_identity.read().await.as_ref(),
            "HOLD: manual inbound target/route missing or changed before journal replay"
        );
        for (uuid, tx) in state.transactions.iter().chain(state.completed.iter()) {
            anyhow::ensure!(
                *uuid == tx.uuid,
                "HOLD: persisted inbound transaction key {uuid} does not match its UUID {}",
                tx.uuid
            );
        }
        anyhow::ensure!(
            state
                .transactions
                .keys()
                .all(|uuid| !state.completed.contains_key(uuid)),
            "HOLD: inbound transaction UUID is owned by both active and completed journals"
        );
        let mut failed = state.failed;
        for mut tx in state.transactions.into_values() {
            if !matches!(tx.status, TxStatus::ComposeProof) {
                anyhow::ensure!(
                    tx.receipt.is_some(),
                    "Transaction {} has no original receipt evidence",
                    tx.uuid
                );
            }
            if matches!(
                tx.status,
                TxStatus::SubmitMessage | TxStatus::PreparingSubmission
            ) {
                let receipt = tx.receipt.as_ref().expect("receipt evidence was validated");
                anyhow::ensure!(
                    receipt.submission_attempts.iter().all(|attempt| {
                        attempt.signed_submission.receipt_key == receipt.receipt_key
                    }),
                    "HOLD: transaction {} archived receipt identity changed",
                    tx.uuid
                );
                // Retry-ready queues retain the first handoff/response in their archived attempts.
                let ambiguous = receipt.signed_submission.is_some()
                    || (receipt.submission_attempts.is_empty()
                        && (receipt.handed_off_at_ms.is_some()
                            || (matches!(tx.status, TxStatus::SubmitMessage)
                                && receipt.initial_response.is_some())));
                if ambiguous {
                    let diagnostic = format!(
                        "Submission for receipt {:?} requires original handoff reconciliation",
                        receipt.receipt_key
                    );
                    tx.status = TxStatus::NeedsReconciliation {
                        diagnostic: diagnostic.clone(),
                    };
                    failed.insert(tx.uuid, diagnostic);
                } else {
                    tx.receipt_event()?;
                }
            }
            if matches!(tx.status, TxStatus::Completed) {
                let diagnostic = "Saved completion requires original finalized dispatch/reply and per-log settlement reconciliation".to_owned();
                tx.status = TxStatus::NeedsReconciliation {
                    diagnostic: diagnostic.clone(),
                };
                failed.insert(tx.uuid, diagnostic);
            }
            tx_manager.add_transaction(tx).await;
        }
        for mut tx in state.completed.into_values() {
            anyhow::ensure!(
                tx.receipt.is_some(),
                "Completed transaction {} has no original receipt evidence",
                tx.uuid
            );
            // Earlier completions could be Processed-only. Requalify the exact
            // original dispatch/reply/effects without resetting discovery or bytes.
            let diagnostic = "Saved completion requires original finalized dispatch/reply and per-log settlement reconciliation".to_owned();
            tx.status = TxStatus::NeedsReconciliation {
                diagnostic: diagnostic.clone(),
            };
            failed.insert(tx.uuid, diagnostic);
            tx_manager.add_transaction(tx).await;
        }
        tx_manager.failed.write().await.extend(failed);

        Ok(true)
    }
    async fn reject_legacy_journal(&self) -> anyhow::Result<()> {
        let mut dir = tokio::fs::read_dir(&self.path).await?;
        while let Some(entry) = dir.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }

            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if name == "failed" || Uuid::parse_str(name).is_ok() {
                anyhow::bail!(
                    "Incompatible transaction journal in '{}': legacy journal layout; expected {STATE_FILE} schema {STATE_SCHEMA_VERSION}",
                    self.path.display()
                );
            }
        }
        Ok(())
    }

    async fn load_blocks(&self) -> anyhow::Result<()> {
        let path = self.path.join("blocks.json");
        let incomplete_path = self.path.join("blocks.json.new");
        match tokio::fs::symlink_metadata(&incomplete_path).await {
                Ok(_) => anyhow::bail!(
                    "Incomplete block storage in '{}': found blocks.json.new; refusing to load an uncommitted snapshot",
                    self.path.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to inspect temporary blocks file in '{}':",
                            self.path.display()
                        )
                    });
                }
            }

        let contents = match tokio::fs::read_to_string(&path).await {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let state_path = self.path.join(STATE_FILE);
                match tokio::fs::symlink_metadata(&state_path).await {
                    Ok(_) => anyhow::bail!(
                        "Incomplete block storage in '{}': state.json exists without blocks.json",
                        self.path.display()
                    ),
                    Err(state_error) if state_error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(());
                    }
                    Err(state_error) => {
                        return Err(state_error).with_context(|| {
                            format!(
                                "Failed to inspect transaction state in '{}':",
                                self.path.display()
                            )
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to read blocks file in '{}':", self.path.display())
                });
            }
        };
        let blocks =
            serde_json::from_str(&contents).context("Failed to deserialize blocks file")?;
        *self.block_storage.blocks.write().await = blocks;
        Ok(())
    }
}

#[async_trait]
impl Storage for JSONStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.block_storage
    }

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        let _guard = self.save_lock.lock().await;
        let identity =
            self.runtime_identity.read().await.clone().ok_or_else(|| {
                anyhow::anyhow!("HOLD: bind inbound runtime identity before saving")
            })?;
        anyhow::ensure!(
            self.path.join(STATE_FILE).exists()
                && !self.path.join(format!("{STATE_FILE}.new")).exists()
                && !self.path.join("blocks.json.new").exists(),
            "HOLD: incomplete or missing inbound transaction/block journal"
        );

        let transactions = tx_manager.transactions.read().await.clone();
        let completed = tx_manager.completed.read().await.clone();
        let failed = tx_manager.failed.read().await.clone();

        self.write_state(&StoredState {
            schema_version: STATE_SCHEMA_VERSION,
            runtime_identity: Some(identity),
            manual_identity: self.manual_identity.read().await.clone(),
            transactions,
            completed,
            failed,
        })
        .await?;

        self.block_storage.save(&self.path).await?;
        Ok(())
    }

    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        if self.path.join(format!("{STATE_FILE}.new")).exists() {
            anyhow::bail!(
                "Incomplete transaction journal in '{}': found {STATE_FILE}.new without a committed state file",
                self.path.display()
            );
        }

        if !self.load_state(tx_manager).await? {
            self.reject_legacy_journal().await?;
            anyhow::ensure!(
                !self.path.join("blocks.json").exists(),
                "HOLD: inbound block snapshot survived without transaction state/identity"
            );
        }
        self.load_blocks().await
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        let _guard = self.save_lock.lock().await;
        anyhow::ensure!(
            self.runtime_identity.read().await.is_some()
                && self.path.join(STATE_FILE).exists()
                && !self.path.join(format!("{STATE_FILE}.new")).exists()
                && !self.path.join("blocks.json.new").exists(),
            "HOLD: inbound block cursor cannot advance without intact bound transaction state"
        );
        self.block_storage().save(&self.path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx_hash(value: u8) -> TxHash {
        TxHash::from([value; 32])
    }

    fn tx_in_slot(slot: u64, tx_hash: TxHash) -> TxHashWithSlot {
        TxHashWithSlot {
            slot_number: EthereumSlotNumber(slot),
            tx_hash,
        }
    }

    fn runtime_identity() -> InboundRuntimeIdentity {
        InboundRuntimeIdentity {
            ethereum_chain_id: 560048,
            ethereum_genesis_hash: H256::repeat_byte(1),
            ethereum_start_block: 70,
            erc20_manager_address: Some(H160::repeat_byte(2)),
            bridging_payment_address: None,
            gear_genesis_hash: H256::repeat_byte(3),
            vft_manager_address: H256::repeat_byte(4),
            checkpoint_light_client_address: H256::repeat_byte(5),
            historical_proxy_address: H256::repeat_byte(6),
            gear_sender: [7; 32],
        }
    }

    #[tokio::test]
    async fn add_block_does_not_reintroduce_completed_transactions() {
        let storage = BlockStorage::new();
        let slot = EthereumSlotNumber(7);
        let block = EthereumBlockNumber(70);
        let tx1 = tx_hash(1);
        let tx2 = tx_hash(2);

        storage
            .add_block(
                slot,
                block,
                primitive_types::H256::repeat_byte(9),
                [tx1, tx2].into_iter(),
            )
            .await
            .unwrap();
        storage.complete_transaction(&tx_in_slot(slot.0, tx1)).await;

        // Replay of the same block should be ignored and not restore tx1.
        storage
            .add_block(
                slot,
                block,
                primitive_types::H256::repeat_byte(9),
                [tx1, tx2].into_iter(),
            )
            .await
            .unwrap();

        assert!(!storage.is_transaction_pending(slot, tx1).await);
        assert!(storage.is_transaction_pending(slot, tx2).await);
    }
    #[tokio::test]
    async fn held_receipt_payload_and_diagnostic_survive_journal_restart() {
        use crate::message_relayer::eth_to_gear::tx_manager::TxStatus;

        let path =
            std::env::temp_dir().join(format!("gear-eth-to-gear-state-{}", uuid::Uuid::now_v7()));
        let storage = JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        let payload = vec![1, 2, 3, 4];
        let diagnostic = "Unknown at finalized block 73 (0xabcd)";
        let mut tx = Transaction::new(
            tx_in_slot(7, tx_hash(9)),
            TxStatus::NeedsReconciliation {
                diagnostic: diagnostic.into(),
            },
        );
        tx.receipt = Some(super::super::tx_manager::ReceiptEvidence {
            payload: payload.clone(),
            receipt_key: (31, 5),
            composed_at_ms: 10,
            handed_off_at_ms: Some(20),
            initial_response: Some(super::super::message_sender::SubmissionEvidence {
                observed_at_ms: 30,
                manager: primitive_types::H256::repeat_byte(1),
                historical_proxy: primitive_types::H256::repeat_byte(2),
                sender: "source-signer".into(),
                manager_reply: Some(vec![9, 8, 7]),
                error: Some("Internal: sharded map error: capacity overflow".into()),
            }),
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        let original = tx.receipt.clone();
        let observation =
            crate::message_relayer::eth_to_gear::message_sender::FinalizedReceiptObservation {
                finalized_block_number: Some(73),
                finalized_block_hash: "0xabcd".into(),
            };
        tx.receipt_observation = Some(observation.clone());
        let uuid = tx.uuid;
        manager.transactions.write().await.insert(uuid, tx);
        manager.failed.write().await.insert(uuid, diagnostic.into());

        storage.save(&manager).await.unwrap();
        let mut extended: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(path.join(STATE_FILE)).await.unwrap()).unwrap();
        extended["operator_extension"] = serde_json::json!({ "owner": "original" });
        extended["transactions"][uuid.to_string()]["receipt"]["original_extension"] =
            serde_json::json!(["retain", 17]);
        tokio::fs::write(
            path.join(STATE_FILE),
            serde_json::to_vec(&extended).unwrap(),
        )
        .await
        .unwrap();
        let latest_diagnostic = "Unknown at finalized block 74 (0xabce)";
        let latest_observation =
            crate::message_relayer::eth_to_gear::message_sender::FinalizedReceiptObservation {
                finalized_block_number: Some(74),
                finalized_block_hash: "0xabce".into(),
            };
        {
            let mut transactions = manager.transactions.write().await;
            let transaction = transactions.get_mut(&uuid).unwrap();
            transaction.receipt_observation = Some(latest_observation.clone());
            if let TxStatus::NeedsReconciliation { diagnostic, .. } = &mut transaction.status {
                *diagnostic = latest_diagnostic.into();
            } else {
                panic!("expected a held transaction before saving updated evidence");
            }
        }
        manager
            .failed
            .write()
            .await
            .insert(uuid, latest_diagnostic.into());
        storage.save(&manager).await.unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(path.join(STATE_FILE)).await.unwrap()).unwrap();
        assert_eq!(saved["operator_extension"], extended["operator_extension"]);
        assert_eq!(
            saved["transactions"][uuid.to_string()]["receipt"]["original_extension"],
            extended["transactions"][uuid.to_string()]["receipt"]["original_extension"]
        );

        let restored = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        storage.load(&restored).await.unwrap();
        let transactions = restored.transactions.read().await;
        match &transactions[&uuid].status {
            TxStatus::NeedsReconciliation {
                diagnostic: stored_diagnostic,
            } => {
                assert_eq!(stored_diagnostic, latest_diagnostic);
            }
            status => panic!("unexpected restored state: {status:?}"),
        }
        assert_eq!(
            transactions[&uuid].receipt_observation.as_ref(),
            Some(&latest_observation)
        );
        assert_eq!(
            serde_json::to_value(&transactions[&uuid].receipt).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        drop(transactions);
        assert_eq!(
            restored.failed.read().await.get(&uuid).map(String::as_str),
            Some(latest_diagnostic)
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn incompatible_or_legacy_transaction_journals_are_rejected() {
        let path = std::env::temp_dir().join(format!(
            "gear-eth-to-gear-old-state-{}",
            uuid::Uuid::now_v7()
        ));
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(
            path.join(STATE_FILE),
            r#"{"schema_version":3,"transactions":{},"completed":{},"failed":{}}"#,
        )
        .await
        .unwrap();
        let storage = JSONStorage::new(&path);
        let manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        assert!(storage.load(&manager).await.is_err());

        tokio::fs::remove_file(path.join(STATE_FILE)).await.unwrap();
        tokio::fs::write(
            path.join(uuid::Uuid::now_v7().to_string()),
            "old transaction",
        )
        .await
        .unwrap();
        assert!(storage.load(&manager).await.is_err());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_save_and_save_blocks_keep_a_complete_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "gear-eth-to-gear-blocks-concurrent-{}",
            uuid::Uuid::now_v7()
        ));
        let storage = std::sync::Arc::new(JSONStorage::new(&path));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = std::sync::Arc::new(TransactionManager::new(std::sync::Arc::new(
            NoStorage::new(),
        )));
        for slot in 0..256 {
            storage
                .block_storage()
                .add_block(
                    EthereumSlotNumber(slot),
                    EthereumBlockNumber(slot * 10),
                    primitive_types::H256::repeat_byte(9),
                    [tx_hash(slot as u8)].into_iter(),
                )
                .await
                .unwrap();
        }
        storage.save_blocks().await.unwrap();
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(9));
        let mut saves = Vec::with_capacity(8);
        for index in 0..8 {
            let storage = storage.clone();
            let manager = manager.clone();
            let start = start.clone();
            saves.push(tokio::spawn(async move {
                start.wait().await;
                if index == 0 {
                    storage.save(&manager).await
                } else {
                    storage.save_blocks().await
                }
            }));
        }
        start.wait().await;
        for save in saves {
            save.await.unwrap().unwrap();
        }
        let saved = tokio::fs::read_to_string(path.join("blocks.json"))
            .await
            .unwrap();
        let blocks: BTreeMap<EthereumSlotNumber, Block> = serde_json::from_str(&saved).unwrap();
        assert_eq!(blocks.len(), 256);
        assert!(blocks.values().all(|block| !block.is_processed()));
        assert!(!path.join("blocks.json.new").exists());
        let restored = JSONStorage::new(&path);
        restored
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let restored_manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        restored.load(&restored_manager).await.unwrap();
        assert_eq!(
            restored.block_storage().blocks_raw().read().await.len(),
            256
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn interrupted_block_snapshot_fails_closed_without_replacing_committed_data() {
        let path = std::env::temp_dir().join(format!(
            "gear-eth-to-gear-blocks-interrupted-{}",
            uuid::Uuid::now_v7()
        ));
        let storage = JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        storage
            .block_storage()
            .add_block(
                EthereumSlotNumber(7),
                EthereumBlockNumber(70),
                primitive_types::H256::repeat_byte(9),
                [tx_hash(7)].into_iter(),
            )
            .await
            .unwrap();
        storage.save_blocks().await.unwrap();
        let committed = tokio::fs::read(path.join("blocks.json")).await.unwrap();
        tokio::fs::write(path.join("blocks.json.new"), b"{truncated")
            .await
            .unwrap();
        let restored = JSONStorage::new(&path);
        let error = restored
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HOLD"));
        assert_eq!(
            tokio::fs::read(path.join("blocks.json")).await.unwrap(),
            committed
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
    #[tokio::test]
    async fn missing_block_snapshot_with_transaction_state_fails_closed() {
        let path = std::env::temp_dir().join(format!(
            "gear-eth-to-gear-blocks-missing-{}",
            uuid::Uuid::now_v7()
        ));
        tokio::fs::create_dir_all(&path).await.unwrap();
        let original = serde_json::to_vec(&serde_json::json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "transactions": {}, "completed": {}, "failed": {},
        }))
        .unwrap();
        tokio::fs::write(path.join(STATE_FILE), &original)
            .await
            .unwrap();
        let storage = JSONStorage::new(&path);
        let result = storage.bind_runtime_identity(runtime_identity()).await;
        let retained = tokio::fs::read(path.join(STATE_FILE)).await.unwrap();
        let created_blocks = path.join("blocks.json").exists();
        tokio::fs::remove_dir_all(path).await.unwrap();
        assert!(result.is_err());
        assert_eq!(retained, original);
        assert!(!created_blocks);
    }
    #[tokio::test]
    async fn fresh_inbound_storage_binds_identity_and_creates_paired_snapshots() {
        let path = std::env::temp_dir().join(format!(
            "gear-eth-to-gear-blocks-empty-{}",
            uuid::Uuid::now_v7()
        ));
        tokio::fs::create_dir_all(&path).await.unwrap();
        let storage = JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        storage.load(&manager).await.unwrap();
        assert!(storage.block_storage().blocks_raw().read().await.is_empty());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn acknowledged_block_with_missing_transaction_state_holds_on_restart() {
        let path = std::env::temp_dir().join(format!("gear-inbound-orphan-{}", Uuid::now_v7()));
        let storage = JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        storage
            .block_storage()
            .add_block(
                EthereumSlotNumber(7),
                EthereumBlockNumber(70),
                primitive_types::H256::repeat_byte(9),
                [tx_hash(9)].into_iter(),
            )
            .await
            .unwrap();
        storage.save_blocks().await.unwrap();
        storage
            .block_storage()
            .complete_transaction(&tx_in_slot(7, tx_hash(9)))
            .await;
        storage.save_blocks().await.unwrap();
        let committed_blocks = tokio::fs::read(path.join("blocks.json")).await.unwrap();
        tokio::fs::remove_file(path.join(STATE_FILE)).await.unwrap();

        let restarted = JSONStorage::new(&path);
        let error = restarted
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HOLD") && error.contains("without transaction state"));
        let manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        assert!(restarted
            .load(&manager)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            tokio::fs::read(path.join("blocks.json")).await.unwrap(),
            committed_blocks
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn deployment_start_and_missing_identity_cannot_rebind_existing_receipt() {
        let path = std::env::temp_dir().join(format!("gear-inbound-identity-{}", Uuid::now_v7()));
        let storage = JSONStorage::new(&path);
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(std::sync::Arc::new(NoStorage::new()));
        let tx = Transaction::new(tx_in_slot(7, tx_hash(9)), TxStatus::ComposeProof);
        let uuid = tx.uuid;
        manager.transactions.write().await.insert(uuid, tx);
        storage.save(&manager).await.unwrap();
        let committed = tokio::fs::read(path.join(STATE_FILE)).await.unwrap();

        let mut changed = runtime_identity();
        changed.ethereum_start_block += 1;
        let restarted = JSONStorage::new(&path);
        assert!(restarted
            .bind_runtime_identity(changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        changed = runtime_identity();
        changed.erc20_manager_address = Some(H160::repeat_byte(9));
        assert!(restarted
            .bind_runtime_identity(changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            tokio::fs::read(path.join(STATE_FILE)).await.unwrap(),
            committed
        );

        let mut unbound: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        unbound.as_object_mut().unwrap().remove("runtime_identity");
        let unbound = serde_json::to_vec(&unbound).unwrap();
        tokio::fs::write(path.join(STATE_FILE), &unbound)
            .await
            .unwrap();
        assert!(restarted
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            tokio::fs::read(path.join(STATE_FILE)).await.unwrap(),
            unbound
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
}
