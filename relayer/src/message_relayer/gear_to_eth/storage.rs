use std::collections::{BTreeMap, HashSet};

use crate::message_relayer::common::{
    gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
    GearBlock,
};
use gsdk::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::U256;
use tokio::sync::RwLock;

#[allow(dead_code)]
pub struct BlockStorage {
    blocks: RwLock<BTreeMap<u32, Block>>,
    n_to_keep: usize,
}

pub struct Block {
    pub number: u32,
    pub transactions: HashSet<U256>,
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

    pub fn blocks_raw(&self) -> &RwLock<BTreeMap<u32, Block>> {
        &self.blocks
    }

    pub async fn complete_transaction(&self, tx: TxWithBlock) {
        let mut blocks = self.blocks.write().await;
        let Some(block) = blocks.get_mut(&tx.block_number) else {
            log::warn!(
                "Block #{} associated with transaction {:?} not found in storage",
                tx.block_number,
                tx.nonce
            );
            return;
        };

        if !block.transactions.remove(&tx.nonce) {
            log::warn!(
                "Transaction {:?} in block #{} is already completed",
                tx.nonce,
                tx.block_number
            );
        }
    }

    pub async fn add_block(&self, block_number: u32, txs: HashSet<U256>) {
        if self
            .blocks
            .write()
            .await
            .insert(
                block_number,
                Block {
                    number: block_number,
                    transactions: txs,
                },
            )
            .is_some()
        {
            log::warn!("Block #{block_number} already exists in storage, overwriting",);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxWithBlock {
    pub block_number: u32,
    pub nonce: U256,
}

pub struct GearRelayerStorage {
    block_storage: BlockStorage,
}

#[async_trait::async_trait]
impl UnprocessedBlocksStorage for GearRelayerStorage {
    async fn add_block(&self, block: &GearBlock) {
        let events = block
            .events()
            .iter()
            .filter_map(|event| match event {
                gclient::Event::GearEthBridge(GearEthBridgeEvent::MessageQueued {
                    message,
                    ..
                }) => {
                    let mut nonce_le = [0; 32];
                    primitive_types::U256(message.nonce.0).to_little_endian(&mut nonce_le);
                    Some(U256::from_little_endian(&nonce_le))
                }
                _ => None,
            })
            .collect::<HashSet<_>>();

        // only add blocks if it contains message queued events.
        if events.is_empty() {
            return;
        }

        log::info!(
            "Queued {} messages in block #{}",
            events.len(),
            block.number()
        );
        self.block_storage.add_block(block.number(), events).await;
    }

    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        todo!()
    }
}
