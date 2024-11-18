// Incorporate code generated based on the IDL file
#[allow(dead_code)]
pub(crate) mod erc20_relay {
    include!(concat!(env!("OUT_DIR"), "/erc20_relay.rs"));
}

#[allow(dead_code)]
pub(crate) mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

use vft::vft_manager::io::SubmitReceipt;

use erc20_relay::{erc_20_relay::io as erc20_relay_io, EthToVaraEvent};
use sails_rs::gstd;

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

    pub fn update_config(&mut self, config_new: Config) {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        state.config = config_new;
    }

    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    pub fn endpoint_for(&mut self, slot: Slot) -> Result<ActorId, ProxyError> {
        self.state.borrow().endpoints.endpoint_for(slot)
    }

    pub fn add_endpoint(&mut self, slot: Slot, endpoint: ActorId) {
        let source = self.exec_context.actor_id();

        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        state.endpoints.push(slot, endpoint);
    }

    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.state.borrow().endpoints.endpoints()
    }

    /// Redirect message to ERC20 Relay service which is valid for `slot`.
    /// If message is relayed successfully then reply from relay service is sent to
    /// `vft_gateway` address and proofs are returned.
    ///
    /// TODO(Adel):
    /// - Accept `Vec<u8>` as `proofs` instead of concrete structure in case erc20-relay changes
    ///   structure of params
    /// - Concrete receipt type instead of `Vec<u8>`?
    pub async fn redirect(
        &mut self,
        slot: Slot,
        vft_manager: ActorId,
        proofs: EthToVaraEvent,
    ) -> Result<Vec<u8>, ProxyError> {
        let state = self.state.borrow();

        let endpoint = state.endpoints.endpoint_for(slot)?;

        let check_proofs = erc20_relay_io::CheckProofs::encode_call(proofs);

        let receipt = Result::<Vec<u8>, erc20_relay::Error>::decode(
            &mut gstd::msg::send_bytes_for_reply(
                endpoint,
                check_proofs,
                0,
                state.config.reply_deposit,
            )
            .map_err(|_| ProxyError::SendFailure)?
            .up_to(Some(state.config.reply_timeout))
            .map_err(|_| ProxyError::ReplyTimeout)?
            .await
            .map_err(|_| ProxyError::ReplyFailure)?
            .as_slice(),
        )
        .map_err(|_| ProxyError::DecodeFailure)?
        .map_err(ProxyError::ERC20Relay)?;
        let state = self.state.borrow();

        let submit_receipt = SubmitReceipt::encode_call(receipt.clone());
        
        let _: () = Result::<(), vft::Error>::decode(
            &mut gstd::msg::send_bytes_for_reply(
                vft_manager,
                submit_receipt,
                0,
                state.config.reply_deposit,
            )
            .map_err(|_| ProxyError::SendFailure)?
            .up_to(Some(state.config.reply_timeout))
            .map_err(|_| ProxyError::ReplyTimeout)?
            .await
            .map_err(|_| ProxyError::ReplyFailure)?
            .as_slice(),
        )
        .map_err(|_| ProxyError::DecodeFailure)?
        .map_err(ProxyError::VftManager)?;
        
        Ok(receipt)
    }
}
