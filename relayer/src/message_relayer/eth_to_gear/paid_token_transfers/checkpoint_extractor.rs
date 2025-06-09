use crate::message_relayer::{
    common::{
        gear::{block_listener::GearBlock, checkpoints_extractor::checkpoints_for_block},
        EthereumSlotNumber,
    },
    eth_to_gear::paid_token_transfers::task_manager::TaskContext,
};
use primitive_types::H256;
pub struct ExtractCheckpoints<'a> {
    pub ctx: &'a TaskContext,
    pub checkpoint_light_client_address: H256,
}

impl<'a> ExtractCheckpoints<'a> {
    pub fn new(ctx: &'a TaskContext, checkpoint_light_client_address: H256) -> Self {
        Self {
            ctx,
            checkpoint_light_client_address,
        }
    }

    /// Run the task by fetching checkpoints from the Gear block
    /// and enqueueing them into the task manager.
    pub async fn run(&mut self, block: &GearBlock) -> anyhow::Result<()> {
        let checkpoints = checkpoints_for_block(block, self.checkpoint_light_client_address);
        if checkpoints.is_empty() {
            log::info!("No checkpoints found for block #{}", block.number());
            return Ok(());
        }
        for (slot, tree_hash_root) in checkpoints {
            log::info!(
                "Found checkpoint: slot={}, tree_hash_root={}",
                slot,
                tree_hash_root
            );
            self.ctx
                .task_manager
                .add_checkpoint(EthereumSlotNumber(slot));
        }
        Ok(())
    }
}
