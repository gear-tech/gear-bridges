// Incorporate code generated based on the IDL file

#[allow(dead_code)]
pub(crate) mod erc20_relay {
    include!(concat!(env!("OUT_DIR"), "/erc20_relay.rs"));
}
use erc20_relay::{traits::*, EthToVaraEvent};
use sails_rs::gstd::{self, calls::GStdRemoting};

use cell::RefCell;
use sails_rs::{calls::Call, gstd::ExecContext, prelude::*};

use crate::{
    error::ProxyError,
    state::{Config, ProxyState, Slot},
};

pub struct HistoricalProxyService<'a, ExecContext> {
    state: &'a RefCell<ProxyState>,
    exec_context: ExecContext,
}

#[sails_rs::service]
impl<'a, T> HistoricalProxyService<'a, T>
where
    T: ExecContext,
{
    pub fn new(state: &'a RefCell<ProxyState>, exec_context: T) -> Self {
        Self {
            state,
            exec_context,
        }
    }

    pub fn config(&self) -> Config {
        self.state.borrow().config
    }

    pub fn update_config(&mut self, config_new: Config) -> Result<(), ProxyError> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            return Err(ProxyError::NotAdmin);
        }

        state.config = config_new;
        Ok(())
    }

    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    pub fn endpoint_for(&mut self, slot: Slot) -> Result<ActorId, ProxyError> {
        self.state.borrow().endpoints.endpoint_for(slot)
    }

    pub fn add_endpoint(&mut self, slot: Slot, endpoint: ActorId) -> Result<(), ProxyError> {
        let source = self.exec_context.actor_id();

        let mut state = self.state.borrow_mut();
        if source != state.admin {
            return Err(ProxyError::NotAdmin);
        }

        state.endpoints.push(slot, endpoint);
        Ok(())
    }

    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.state.borrow().endpoints.endpoints()
    }

    /// Redirect message to ERC20 Relay service which is valid for `slot`.
    /// If message is relayed successfully then reply from relay service is sent to
    /// `vft_gateway` address and proofs are returned.
    ///
    /// # Parameters
    ///
    /// - `slot`: slot for which message is relayed.
    /// - `tx_index`: transaction index for message.
    /// - `proofs`: SCALE encoded `EthToVaraEvent`.
    /// - `client`: client address to send receipt to on success.
    /// - `client_route`: route to send receipt to on success.
    /// # Returns
    /// - `(Vec<u8>, Vec<u8>)`: on success where first vector is receipt and second vector is reply from calling `client_route`.
    /// - `ProxyError`: if redirect failed
    ///
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn redirect(
        &mut self,
        slot: Slot,
        tx_index: u64,
        proofs: Vec<u8>,
        client: ActorId,
        client_route: Vec<u8>,
    ) -> Result<(Vec<u8>, Vec<u8>), ProxyError> {
        let state = self.state.borrow();
        let reply_deposit = state.config.reply_deposit;
        let reply_timeout = state.config.reply_timeout;
        let endpoint = state.endpoints.endpoint_for(slot)?;
        drop(state);
        // 1) Check if proofs are correct in erc20-relay
        let receipt = erc20_relay::Erc20Relay::new(GStdRemoting)
            .check_proofs(
                EthToVaraEvent::decode(&mut proofs.as_slice())
                    .map_err(|_| ProxyError::DecodeFailure)?,
            )
            .send_recv(endpoint)
            .await
            .map_err(|_| ProxyError::ReplyFailure)?
            .map_err(ProxyError::ERC20Relay)?;

        // 2) Invoke client with a receipt. Uses route and address suplied by the user.
        let submit_receipt = {
            let params = (slot, tx_index, receipt.clone());
            let mut payload = Vec::with_capacity(params.encoded_size() + client_route.len());
            payload.extend_from_slice(&client_route);
            params.encode_to(&mut payload);
            payload
        };

        let reply = gstd::msg::send_bytes_for_reply(client, submit_receipt, 0, reply_deposit)
            .map_err(|_| ProxyError::SendFailure)?
            .up_to(Some(reply_timeout))
            .map_err(|_| ProxyError::ReplyTimeout)?
            .await
            .map_err(|_| ProxyError::ReplyFailure)?;

        Ok((receipt, reply))
    }
}
