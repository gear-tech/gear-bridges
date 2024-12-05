#![no_std]

pub mod error;
pub mod service;

use cell::RefCell;
use sails_rs::{
    gstd::{ExecContext, GStdExecContext},
    prelude::*,
};
use service::Erc20Relay as Erc20RelayService;

pub struct State {
    admin: ActorId,
    checkpoint_light_client_address: ActorId,
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(checkpoint_light_client_address: ActorId) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoint_light_client_address,
        }))
    }

    pub fn erc20_relay(&self) -> Erc20RelayService {
        Erc20RelayService::new(&self.0)
    }
}
