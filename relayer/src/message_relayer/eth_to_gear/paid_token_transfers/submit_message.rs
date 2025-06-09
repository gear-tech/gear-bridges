use crate::message_relayer::{
    common::TxHashWithSlot, eth_to_gear::paid_token_transfers::task_manager::TaskContext,
};
use eth_events_electra_client::EthToVaraEvent;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use sails_rs::{
    calls::{Action, ActionIo, Call},
    gclient::calls::GClientRemoting,
    Encode,
};
use vft_manager_client::vft_manager::io::SubmitReceipt;

pub struct SubmitMessageTask<'a> {
    ctx: &'a TaskContext,
    payload: EthToVaraEvent,
    tx: TxHashWithSlot,
    historical_proxy_address: H256,
    vft_manager_address: H256,
    suri: String,
}

impl<'a> SubmitMessageTask<'a> {
    pub fn new(
        ctx: &'a TaskContext,
        payload: EthToVaraEvent,
        tx: TxHashWithSlot,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        suri: String,
    ) -> Self {
        Self {
            ctx,
            payload,
            tx,
            historical_proxy_address,
            vft_manager_address,
            suri,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let gear_api = self.ctx.gclient_api(&self.suri)?;
        let gas_limit_block = gear_api.block_gas_limit()?;
        let gas_limit = gas_limit_block / 100 * 95;

        let remoting = GClientRemoting::new(gear_api.clone());
        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();
        let mut proxy_service = HistoricalProxy::new(remoting.clone());

        let (_, receiver_reply) = proxy_service
            .redirect(
                self.payload.proof_block.block.slot,
                self.payload.encode(),
                self.vft_manager_address.into(),
                route,
            )
            .with_gas_limit(gas_limit)
            .send_recv(self.historical_proxy_address.into())
            .await
            .map_err(|err| anyhow::anyhow!("Failed to send message to historical proxy: {err:?}"))?
            .map_err(|err| {
                anyhow::anyhow!("Failed to receive reply from historical proxy: {err:?}")
            })?;

        let reply = SubmitReceipt::decode_reply(&receiver_reply)
            .map_err(|err| anyhow::anyhow!("Failed to decode vft-manager reply: {err}"))?;

        match reply {
            Ok(_) => Ok(()),
            Err(vft_manager_client::Error::NotSupportedEvent) => {
                log::warn!("Dropping message for {} as it's considered invalid by vft-manager (probably unsupported ERC20 token)", self.tx.tx_hash);
                Ok(())
            }

            Err(err) => Err(anyhow::anyhow!("Internal vft-manager error: {err:?}")),
        }
    }
}
