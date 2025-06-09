use crate::message_relayer::{
    common::{gear::message_sender::compose_payload, TxHashWithSlot},
    eth_to_gear::paid_token_transfers::task_manager::{Task, TaskContext},
};
use primitive_types::H256;

pub struct ProofComposerTask<'a> {
    pub ctx: &'a TaskContext,
    pub message: TxHashWithSlot,
    pub historical_proxy_address: H256,
    pub suri: String,
}

impl<'a> ProofComposerTask<'a> {
    pub fn new(
        ctx: &'a TaskContext,
        message: TxHashWithSlot,
        historical_proxy_address: H256,
        suri: String,
    ) -> Self {
        Self {
            ctx,
            message,
            historical_proxy_address,
            suri,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let gear_api = self.ctx.gclient_api(&self.suri)?;
        let payload = compose_payload::compose(
            &self.ctx.beacon_client(),
            &gear_api,
            &self.ctx.eth_api(),
            self.message.tx_hash,
            self.historical_proxy_address.into(),
        )
        .await?;

        log::info!(
            "Sending message in gear_message_sender: tx_index={}, slot={}",
            payload.transaction_index,
            payload.proof_block.block.slot
        );

        self.ctx
            .task_manager
            .enqueue(Task::submit_message(payload, self.message));
        Ok(())
    }
}
