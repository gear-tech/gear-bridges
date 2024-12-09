#![no_std]

pub mod error;
pub mod service;

use cell::RefCell;
use sails_rs::{
    gstd::{ExecContext, GStdExecContext},
    prelude::*,
};
use service::EthereumEventClient as EthereumEventClientService;

pub struct State {
    admin: ActorId,
    checkpoint_light_client_address: ActorId,
}

pub struct EthereumEventClientProgram(RefCell<State>);

#[sails_rs::program]
impl EthereumEventClientProgram {
    pub fn new(checkpoint_light_client_address: ActorId) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoint_light_client_address,
        }))
    }

    pub fn ethereum_event_client(&self) -> EthereumEventClientService {
        EthereumEventClientService::new(&self.0)
    }
}
