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
    pub async fn redirect(
        &mut self,
        slot: Slot,
        _vft_gateway: ActorId,
        _proofs: Vec<u8>,
    ) -> Result<Vec<u8>, ProxyError> {
        let state = self.state.borrow();

        let _endpoint = state.endpoints.endpoint_for(slot);
        // TODO: Send request to `endpoint` and if proofs are correct then send mesage to `vft_gateway`
        todo!()
    }
}
