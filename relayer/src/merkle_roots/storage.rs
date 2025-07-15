use crate::message_relayer::common::gear::{
    block_listener::GearBlock,
    block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
};
use gclient::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
};

pub struct MerkleRootBlockStorage {
    pub blocks: RwLock<BTreeMap<u32, Block>>,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Block {
    pub block_hash: H256,
    pub merkle_root_changed: Option<H256>,
    pub authority_set_hash_changed: Option<H256>,
}

fn queue_merkle_root_changed(block: &GearBlock) -> Option<H256> {
    block.events().iter().find_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::QueueMerkleRootChanged(hash)) => {
            Some(*hash)
        }
        _ => None,
    })
}

fn authority_set_changed(block: &GearBlock) -> Option<H256> {
    block.events().iter().find_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::AuthoritySetHashChanged(hash)) => {
            Some(*hash)
        }
        _ => None,
    })
}

#[async_trait::async_trait]
impl UnprocessedBlocksStorage for MerkleRootBlockStorage {
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        let blocks = self.blocks.read().await;
        let first_block = blocks.first_key_value().map(|(k, v)| (v.block_hash, *k));
        UnprocessedBlocks {
            blocks: blocks.iter().map(|(k, v)| (v.block_hash, *k)).collect(),
            last_block: None,
            first_block,
        }
    }

    async fn add_block(&self, block: &GearBlock) {
        let merkle_root_changed = queue_merkle_root_changed(block);
        let authority_set_hash_changed = authority_set_changed(block);

        // in case there are no merkle-root related changes we can just skip the block saving.
        if merkle_root_changed.is_none() && authority_set_hash_changed.is_none() {
            return;
        }

        let block_hash = block.hash();
        let block_number = block.number();

        let mut blocks = self.blocks.write().await;
        match blocks.entry(block_number) {
            Entry::Occupied(_) => {
                log::warn!("Block #{block_number} already exists in storage");
                return;
            }

            Entry::Vacant(entry) => {
                entry.insert(Block {
                    block_hash,
                    merkle_root_changed,
                    authority_set_hash_changed,
                });
            }
        }
    }
}

impl MerkleRootBlockStorage {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            blocks: RwLock::new(BTreeMap::new()),
            path,
        })
    }

    pub async fn merkle_root_processed(&self, block_number: u32) {
        let mut blocks = self.blocks.write().await;

        let Entry::Occupied(entry) = blocks.entry(block_number).and_modify(|block| {
            block.merkle_root_changed = None;
        }) else {
            return;
        };

        if entry.get().authority_set_hash_changed.is_none() {
            entry.remove();
        }
    }

    pub async fn authority_set_processed(&self, block_number: u32) {
        let mut blocks = self.blocks.write().await;

        let Entry::Occupied(entry) = blocks.entry(block_number).and_modify(|block| {
            block.authority_set_hash_changed = None;
        }) else {
            return;
        };

        if entry.get().merkle_root_changed.is_none() {
            entry.remove();
        }
    }

    /// Save unprocessed blocks to the provided path.
    pub async fn save(&self) -> anyhow::Result<()> {
        let blocks = self.blocks.read().await;

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .await?;

        let serialized = serde_json::to_string(&*blocks)?;

        file.write_all(serialized.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .open(&self.path)
            .await?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).await?;

        let blocks: BTreeMap<u32, Block> = serde_json::from_str(&contents)?;
        *self.blocks.write().await = blocks;

        Ok(())
    }
}
