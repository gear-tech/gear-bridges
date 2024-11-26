// Incorporate code generated based on the IDL file

#[allow(dead_code)]
pub(crate) mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

#[allow(dead_code)]
pub(crate) mod erc20_relay {
    include!(concat!(env!("OUT_DIR"), "/erc20_relay.rs"));
}

use sails_rs::{calls::ActionIo, gstd};
use vft::vft_manager::io::SubmitReceipt;

use cell::RefCell;
use sails_rs::{gstd::ExecContext, prelude::*};

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
    /// - `slot`: slot for which message is relayed
    /// - `vft_manager`: a Vft manager address to submit receipt to if erc20-relay confirms proofs are correct
    /// - `proofs`: raw SCALE-encoded call to `CheckProofs` of erc20-relay.
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn redirect(
        &mut self,
        slot: Slot,
        vft_manager: ActorId,
        proofs: Vec<u8>,
    ) -> Result<Vec<u8>, ProxyError> {
        let state = self.state.borrow();

        let endpoint = state.endpoints.endpoint_for(slot)?;
        let reply_deposit = state.config.reply_deposit;
        let reply_timeout = state.config.reply_timeout;
        let mut check_proofs =
            <erc20_relay::erc_20_relay::io::CheckProofs as ActionIo>::ROUTE.to_vec();
        check_proofs.extend_from_slice(&proofs);
        let reply = gstd::msg::send_bytes_for_reply(endpoint, check_proofs, 0, 0)
            .map_err(|_| ProxyError::SendFailure)?
            .up_to(Some(reply_timeout))
            .map_err(|_| ProxyError::ReplyTimeout)?
            .await
            .map_err(|_| ProxyError::ReplyFailure)?;

        let receipt = <erc20_relay::erc_20_relay::io::CheckProofs as ActionIo>::decode_reply(reply)
            .map_err(|_| ProxyError::DecodeFailure)?
            .map_err(ProxyError::ERC20Relay)?;

        let submit_receipt = SubmitReceipt::encode_call(receipt.clone());

        let reply = gstd::msg::send_bytes_for_reply(vft_manager, submit_receipt, 0, reply_deposit)
            .map_err(|_| ProxyError::SendFailure)?
            .up_to(Some(reply_timeout))
            .map_err(|_| ProxyError::ReplyTimeout)?
            .await
            .unwrap();
        // return error if we cannot submit receipt.
        SubmitReceipt::decode_reply(&reply)
            .map_err(|_| ProxyError::DecodeFailure)?
            .map_err(ProxyError::VftManager)?;

        Ok(receipt)
    }
}
