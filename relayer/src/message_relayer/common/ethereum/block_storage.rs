use async_trait::async_trait;
// use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};
use tokio::sync::RwLock;

/// Storage for unprocessed ethereum blocks.
#[async_trait]
pub trait UnprocessedBlockStorage: Send + Sync {
    /// Returns list of unprocessed blocks.
    async fn unprocessed_blocks(&self) -> anyhow::Result<Vec<u64>>;
    /// Adds a block to the storage.
    async fn add_block(&self, block: u64) -> anyhow::Result<()>;
}

pub struct NoStorage;

#[async_trait]
impl UnprocessedBlockStorage for NoStorage {
    async fn unprocessed_blocks(&self) -> anyhow::Result<Vec<u64>> {
        Ok(vec![])
    }

    async fn add_block(&self, _block: u64) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct JSONBlockStorage {
    path: PathBuf,
    blocks: RwLock<HashSet<u64>>,
}

impl JSONBlockStorage {
    pub async fn new(path: PathBuf) -> anyhow::Result<Self> {
        let blocks = if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashSet::new()
        };

        Ok(Self {
            path,
            blocks: RwLock::new(blocks),
        })
    }

    async fn save(&self) -> anyhow::Result<()> {
        let blocks = self.blocks.read().await;
        // Use a temporary file to ensure atomic writes
        let temp_path = self.path.with_extension("tmp");
        let content = serde_json::to_string(&*blocks)?;
        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(temp_path, &self.path).await?;
        Ok(())
    }
}

#[async_trait]
impl UnprocessedBlockStorage for JSONBlockStorage {
    async fn unprocessed_blocks(&self) -> anyhow::Result<Vec<u64>> {
        let blocks = self.blocks.read().await;
        let mut sorted_blocks: Vec<u64> = blocks.iter().cloned().collect();
        sorted_blocks.sort();
        Ok(sorted_blocks)
    }

    async fn add_block(&self, block: u64) -> anyhow::Result<()> {
        {
            let mut blocks = self.blocks.write().await;
            if !blocks.insert(block) {
                return Ok(());
            }
        }
        self.save().await
    }
}
