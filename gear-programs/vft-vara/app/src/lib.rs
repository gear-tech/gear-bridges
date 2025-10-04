#![no_std]

use awesome_sails_services::{
    vft::{
        self,
        utils::{Allowances, Balances},
    },
    vft_admin::{
        self,
        utils::{Pausable, Pause},
        Authorities,
    },
    vft_extension,
    vft_metadata::{self, Metadata},
    vft_native_exchange, vft_native_exchange_admin,
};
use core::cell::RefCell;
use sails_rs::{gstd::msg, prelude::*};

/// Specifies the network for deployment of VFT-VARA
#[derive(Decode, TypeInfo)]
#[scale_info(crate = sails_rs::scale_info)]
#[codec(crate = sails_rs::scale_codec)]
pub enum Mainnet {
    Yes,
    No,
}

pub struct Program {
    authorities: RefCell<Authorities>,
    allowances: Pausable<RefCell<Allowances>>,
    balances: Pausable<RefCell<Balances>>,
    metadata: RefCell<Metadata>,
    pause: Pause,
}

#[program]
impl Program {
    pub fn new(network: Mainnet) -> Self {
        let pause = Pause::default();

        // Allowance is represented as 9 bytes unsigned int.
        // The maximum value is 4,722,366,482,869,645,213,695.
        let mut allowances = Allowances::default();
        // 24h / 3sec block
        allowances.set_expiry_period(24 * 60 * 60 / 3);

        // Minimum balance is zero by default.
        //
        // Balance is represented as 10 bytes unsigned int.
        // The maximum value is 1,208,925,819,614,629,174,706,175.
        let mut balances = Balances::default();
        balances.set_minimum_balance(1_000_000_000_000u64.into());

        let metadata = match network {
            Mainnet::Yes => Metadata::new("Wrapped Vara".into(), "WVARA".into(), 12),

            Mainnet::No => Metadata::new("Wrapped Testnet Vara".into(), "WTVARA".into(), 12),
        };

        Self {
            authorities: RefCell::new(Authorities::from_one(msg::source())),
            allowances: Pausable::new(&pause, RefCell::new(allowances)),
            balances: Pausable::new(&pause, RefCell::new(balances)),
            metadata: RefCell::new(metadata),
            pause,
        }
    }

    #[allow(dead_code)]
    #[handle_reply]
    fn handle_reply(&self) {
        self.vft_native_exchange_admin().handle_reply()
    }

    pub fn vft(&self) -> vft_common::Service<'_> {
        vft_common::Service::new(&self.allowances, &self.balances, &self.metadata)
    }

    pub fn vft2(&self) -> vft::Service<'_> {
        vft::Service::new(&self.allowances, &self.balances)
    }

    pub fn vft_admin(&self) -> vft_admin::Service<'_> {
        vft_admin::Service::new(
            &self.authorities,
            &self.allowances,
            &self.balances,
            &self.pause,
            self.vft2().emitter(),
        )
    }

    pub fn vft_extension(&self) -> vft_extension::Service<'_> {
        vft_extension::Service::new(&self.allowances, &self.balances, self.vft2().emitter())
    }

    pub fn vft_metadata(&self) -> vft_metadata::Service<'_> {
        vft_metadata::Service::new(&self.metadata)
    }

    pub fn vft_native_exchange(&self) -> vft_native_exchange::Service<'_> {
        vft_native_exchange::Service::new(&self.balances, self.vft2().emitter())
    }

    pub fn vft_native_exchange_admin(&self) -> vft_native_exchange_admin::Service<'_> {
        vft_native_exchange_admin::Service::new(self.vft_admin())
    }
}
