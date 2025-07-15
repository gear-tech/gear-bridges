use primitive_types::H256;

use crate::message_relayer::common::gear::block_listener::GearBlock;

/// A list of unprocessed blocks and also first/last blocks in the storage.
///
/// For merkle-root relayer we must fetch all blocks from `first_block` to the latest finalized block on chain.
/// For relayers we can fetch blocks from `last_block` to the latest finalized block on chain.
pub struct UnprocessedBlocks {
    pub blocks: Vec<(H256, u32)>,
    /// First block in the storage that is not processed yet.
    pub first_block: Option<(H256, u32)>,
    /// Last block in the storage that is not processed yet.
    pub last_block: Option<(H256, u32)>,
}

/// Trait that defines interface for BlockListener to access unprocessed blocks.
#[async_trait::async_trait]
pub trait UnprocessedBlocksStorage: Send + Sync {
    /// Returns unprocessed blocks from the storage.
    ///
    /// Note that you have to choose whether to return `first_block` or `last_block` based on the relayer type.
    /// If both are `Some` then `first_block` will be preferred as a starting point for re-fetching. If both of them
    /// are `None` then there will be no re-fetching of blocks from RPC.
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks;
    /// Adds a block to the storage. Implementer of the trait
    /// is responsible for processing the block later on.
    async fn add_block(&self, block: &GearBlock);
}

pub struct NoStorage;

#[async_trait::async_trait]
impl UnprocessedBlocksStorage for NoStorage {
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        UnprocessedBlocks {
            blocks: vec![],
            last_block: None,
            first_block: None,
        }
    }

    async fn add_block(&self, _block: &GearBlock) {}
}
