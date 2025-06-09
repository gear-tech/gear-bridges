use ethereum_client::FeePaidEntry;
use primitive_types::H160;

use crate::message_relayer::{
    common::{EthereumBlockNumber, EthereumSlotNumber},
    eth_to_gear::paid_token_transfers::task_manager::{Task, TaskContext},
};

/// A task to extract paid message events from a single Ethereum block.
pub struct ExtractMessagePaidEvents<'a> {
    ctx: &'a TaskContext,
    block: EthereumBlockNumber,
    slot_number: EthereumSlotNumber,
    bridging_payment_address: H160,
    // Other fields can be added as needed
}

impl<'a> ExtractMessagePaidEvents<'a> {
    pub fn new(
        ctx: &'a TaskContext,

        block: EthereumBlockNumber,
        slot_number: EthereumSlotNumber,
        bridging_payment_address: H160,
    ) -> Self {
        Self {
            ctx,
            block,
            slot_number,
            bridging_payment_address,
        }
    }

    /// Run the task by fetching paid events from the Ethereum API
    /// and enqueueing them into the task manager.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let events = self
            .ctx
            .eth_api
            .as_ref()
            .unwrap()
            .fetch_fee_paid_events(self.bridging_payment_address, self.block.0)
            .await?;

        if events.is_empty() {
            log::info!("No paid events found for block {}", self.block);
            return Ok(());
        }
        let slot_number = self.slot_number;
        for FeePaidEntry { tx_hash } in events {
            log::info!("Found fee paid event: tx_hash={tx_hash}, slot_number={slot_number}");
            self.ctx
                .task_manager
                .enqueue(Task::paid_event(tx_hash, slot_number));
        }
        Ok(())
    }
}
