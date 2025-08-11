#![allow(dead_code, unused_variables)]
use anyhow::Context;
use primitive_types::H256;
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

use crate::message_relayer::{
    common::{
        ethereum::accumulator::utils::MerkleRoots,
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock, GearBlockNumber, MessageInBlock,
    },
    gear_to_eth::tx_manager::{Transaction, TransactionManager},
};

pub struct BlockStorage {
    blocks: RwLock<BTreeMap<GearBlockNumber, Block>>,
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
            n_to_keep: 100,
        }
    }

    pub async fn save(&self, path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    pub async fn complete_transaction(&self, message: &MessageInBlock) {
        let mut blocks = self.blocks.write().await;
        let Some(block) = blocks.get_mut(&message.block) else {
            return;
        };

        block.messages.remove(&message.message.nonce_le);
    }

    pub async fn add_block(
        &self,
        block: GearBlockNumber,
        block_hash: H256,
        txs: impl Iterator<Item = [u8; 32]>,
    ) {
        let mut blocks = self.blocks.write().await;

        blocks.insert(
            block,
            Block {
                block_hash,
                messages: txs.collect(),
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

        let last_block = blocks
            .last_key_value()
            .map(|(k, block)| (block.block_hash, k.0));

        UnprocessedBlocks {
            blocks: unprocessed,
            last_block,
            first_block: None,
        }
    }
}

pub struct Block {
    pub block_hash: H256,
    pub messages: HashSet<[u8; 32]>,
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
}

#[async_trait::async_trait]
impl<T: Storage> UnprocessedBlocksStorage for T {
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        self.block_storage().unprocessed_blocks().await
    }

    async fn add_block(&self, block: &GearBlock) {
        let _ = block;
    }
}

pub struct NoStorage(BlockStorage);

impl Default for NoStorage {
    fn default() -> Self {
        Self(BlockStorage {
            blocks: RwLock::new(BTreeMap::new()),
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
                        let _ = tx_manager.merkle_roots.write().await.add(*root);
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
