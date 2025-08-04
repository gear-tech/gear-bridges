#![allow(dead_code, unused_variables)]
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::message_relayer::{
    common::{ethereum::accumulator::utils::MerkleRoots, GearBlock},
    gear_to_eth::tx_manager::{Transaction, TransactionManager},
};

pub struct BlockStorage {
    blocks: BTreeMap<u32, GearBlock>,
    n_to_keep: usize,
}

impl BlockStorage {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            n_to_keep: 100,
        }
    }

    pub async fn save(&self, path: &Path) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct Block {
    pub messages: HashSet<u64>,
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    fn block_storage(&self) -> &BlockStorage;

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn save_blocks(&self) -> anyhow::Result<()>;
}

pub struct NoStorage(BlockStorage);

impl Default for NoStorage {
    fn default() -> Self {
        Self(BlockStorage {
            blocks: BTreeMap::new(),
            n_to_keep: 0,
        })
    }
}

impl NoStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

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
}

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
            .write(true)
            .create(true)
            .truncate(true)
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
            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in file name: {tx_file:?}"))?;

        let mut contents = String::new();

        let mut file = tokio::fs::File::open(path.join(&tx_file))
            .await
            .with_context(|| format!("Failed to open transaction file: {tx_file:?}"))?;

        file.read_to_string(&mut contents)
            .await
            .with_context(|| format!("Failed to read transaction file: {tx_file:?}"))?;

        let tx: Transaction = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to deserialize transaction from file: {tx_file:?}"))?;

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

        let mut failed = BTreeMap::new();
        let failed_map = tx_manager.failed.read().await;

        for (tx_uuid, tx) in tx_manager.transactions.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
            if let Some(reason) = failed_map.get(tx_uuid) {
                failed.insert(*tx_uuid, reason.clone());
            }
        }

        for (tx_uuid, tx) in tx_manager.completed.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
        }

        let mut failed_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.path.join("failed"))
            .await?;

        let str = serde_json::to_string(&failed)
            .with_context(|| "Failed to serialize failed transactions")?;

        failed_file
            .write_all(str.as_bytes())
            .await
            .with_context(|| "Failed to write failed transactions to file")?;

        failed_file.flush().await?;

        let merkle = tx_manager.merkle_roots.read().await;
        let mut merkle_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.path.join("merkle_roots"))
            .await?;

        let str =
            serde_json::to_string(&*merkle).with_context(|| "Failed to serialize merkle roots")?;
        merkle_file
            .write_all(str.as_bytes())
            .await
            .with_context(|| "Failed to write merkle roots to file")?;
        merkle_file.flush().await?;

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
                    let contents = tokio::fs::read_to_string(entry.path())
                        .await
                        .context("Failed to read 'failed' transactions file")?;
                    let map: BTreeMap<Uuid, String> = serde_json::from_str(&contents)
                        .context("Failed to parse 'failed' transactions")?;
                    tx_manager.failed.write().await.extend(map);
                } else if entry.file_name().to_str() == Some("merkle_roots") {
                    let contents = tokio::fs::read_to_string(entry.path())
                        .await
                        .context("Failed to read 'merkle_roots' file")?;
                    let merkle_roots: MerkleRoots = serde_json::from_str(&contents)
                        .context("Failed to parse 'merkle_roots'")?;
                    for i in 0..merkle_roots.len() {
                        let root = merkle_roots.get(i).expect("Root should exist");
                        let _ = tx_manager.merkle_roots.write().await.add(root.clone());
                    }
                } else if entry.file_name().to_str() == Some("blocks") {
                } else {
                    let tx = self.read_tx(entry.path(), entry.file_name()).await?;

                    tx_manager.add_transaction(tx).await;
                }
            }
        }

        Ok(())
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        // Implement saving blocks logic here
        Ok(())
    }
}
