use super::tx_manager::{Transaction, TransactionManager};
use crate::message_relayer::common::{EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot};
use anyhow::Context;
use async_trait::async_trait;
use ethereum_client::TxHash;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
};
use uuid::Uuid;

/// Storage type implementing
/// storage for Ethereum blocks.
pub struct BlockStorage {
    blocks: RwLock<BTreeMap<EthereumSlotNumber, Block>>,
    n_to_keep: usize,
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    pub number: EthereumBlockNumber,
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

    pub async fn add_block(
        &self,
        slot: EthereumSlotNumber,
        number: EthereumBlockNumber,
        txs: impl Iterator<Item = TxHash>,
    ) {
        if self
            .blocks
            .write()
            .await
            .insert(
                slot,
                Block {
                    number,
                    transactions: txs.collect(),
                },
            )
            .is_some()
        {
            log::warn!("Block at slot #{} is already in storage", slot.0);
        };
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
        if blocks_old.exists() {
            tokio::fs::remove_file(&blocks_old).await.with_context(|| {
                format!(
                    "Failed to remove old blocks file in storage path: '{}'",
                    path.display()
                )
            })?;
        }
        tokio::fs::rename(blocks_new, blocks_old)
            .await
            .with_context(|| {
                format!(
                    "Failed to rename new blocks file in storage path: '{}'",
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

pub struct NoStorage(BlockStorage);

impl Default for NoStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl NoStorage {
    pub fn new() -> Self {
        Self(BlockStorage::new())
    }
}

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
}

impl JSONStorage {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            block_storage: BlockStorage::new(),
        }
    }

    async fn write_tx(&self, tx_uuid: &Uuid, tx: &Transaction) -> anyhow::Result<()> {
        let filename = self.path.join(tx_uuid.to_string());
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(filename)
            .await?;

        let json = serde_json::to_string(&tx)?;
        file.write_all(json.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    async fn read_tx(&self, path: PathBuf, tx_file: OsString) -> anyhow::Result<Transaction> {
        let uuid = tx_file
            .to_str()
            .and_then(|s| Uuid::from_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in filename {tx_file:?}"))?;
        let mut contents = String::new();

        let mut file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("Failed to open transaction file: {tx_file:?}"))?;
        file.read_to_string(&mut contents)
            .await
            .with_context(|| format!("Failed to read transaction file: {tx_file:?}"))?;

        let tx: Transaction = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to deserialize transaction from file: {tx_file:?}"))?;

        if tx.uuid != uuid {
            return Err(anyhow::anyhow!(
                "UUID in filename does not match transaction UUID: {} != {}",
                uuid,
                tx.uuid
            ));
        }

        Ok(tx)
    }
}

#[async_trait]
impl Storage for JSONStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.block_storage
    }

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            tokio::fs::create_dir_all(&self.path).await?;
        }
        let mut failed = BTreeMap::new();

        let failed_map = tx_manager.failed.read().await;
        for (tx_uuid, tx) in tx_manager.transactions.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
            if let Some(reason) = failed_map.get(tx_uuid) {
                failed.insert(*tx_uuid, reason);
            }
        }

        for (tx_uuid, tx) in tx_manager.completed.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
        }

        let mut failed_file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.path.join("failed"))
            .await
            .with_context(|| {
                format!(
                    "Failed to open or create 'failed' file in storage path: '{}'",
                    self.path.display()
                )
            })?;

        let str = serde_json::to_string(&failed)?;
        failed_file
            .write_all(str.as_bytes())
            .await
            .with_context(|| {
                format!(
                    "Failed to write failed transactions to 'failed' file in storage path: '{}'",
                    self.path.display()
                )
            })?;
        failed_file.flush().await?;
        self.block_storage.save(&self.path).await?;

        Ok(())
    }

    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        let mut dir = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = dir.next_entry().await? {
            if entry
                .file_type()
                .await
                .context("directory entry is unaccessible")?
                .is_file()
            {
                if entry.file_name().to_str() == Some("failed") {
                    let contents =
                        tokio::fs::read_to_string(entry.path())
                            .await
                            .with_context(|| {
                                format!(
                                    "Failed to read 'failed' file in storage path: '{}'",
                                    self.path.display()
                                )
                            })?;
                    let map: BTreeMap<Uuid, String> = serde_json::from_str(&contents)?;
                    *tx_manager.failed.write().await = map;
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
                    let map: BTreeMap<EthereumSlotNumber, Block> = serde_json::from_str(&contents)?;
                    *self.block_storage().blocks.write().await = map;
                } else {
                    let tx = self.read_tx(entry.path(), entry.file_name()).await?;

                    tx_manager.add_transaction(tx).await;
                }
            }
        }

        Ok(())
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        self.block_storage().save(&self.path).await
    }
}
