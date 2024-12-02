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
    vft_manager: ActorId,
    config: Config,
}

#[derive(Clone, Copy, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Config {
    reply_timeout: u32,
    reply_deposit: u64,
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(checkpoint_light_client_address: ActorId, config: Config) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoint_light_client_address,
            vft_manager: Default::default(),
            config,
        }))
    }

    pub fn erc20_relay(&self) -> Erc20RelayService<GStdExecContext> {
        Erc20RelayService::new(&self.0, GStdExecContext::new())
    }
}
