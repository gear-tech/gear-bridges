#![no_std]

pub mod error;
pub mod service;

use cell::RefCell;
use collections::BTreeSet;
use sails_rs::{
    gstd::{ExecContext, GStdExecContext},
    prelude::*,
};
use service::Erc20Relay as Erc20RelayService;

pub struct State {
    admin: ActorId,
    checkpoint_light_client_address: ActorId,
    vft_gateway: ActorId,
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
        unsafe {
            service::TRANSACTIONS = Some(BTreeSet::new());
        }

        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoint_light_client_address,
            vft_gateway: Default::default(),
            config,
        }))
    }

    pub fn gas_calculation(_reply_timeout: u32, _reply_deposit: u64) -> Self {
        #[cfg(feature = "gas_calculation")]
        {
            let self_ = Self::new(
                Default::default(),
                Config {
                    reply_timeout: _reply_timeout,
                    reply_deposit: _reply_deposit,
                },
            );

            let transactions = service::transactions_mut();
            for i in 0..service::CAPACITY_STEP_SIZE {
                transactions.insert((0, i as u64));
            }

            self_
        }

        #[cfg(not(feature = "gas_calculation"))]
        panic!("Please rebuild with enabled `gas_calculation` feature")
    }

    pub fn erc20_relay(&self) -> Erc20RelayService<GStdExecContext> {
        Erc20RelayService::new(&self.0, GStdExecContext::new())
    }
}
