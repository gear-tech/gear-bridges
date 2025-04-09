#![no_std]

use awesome_sails_services::{
    vft::{
        self,
        utils::{Allowances, Balances},
    },
    vft_admin::{
        self, Authorities,
        utils::{Pausable, Pause},
    },
    vft_extension,
    vft_metadata::{self, Metadata},
};
use core::cell::RefCell;
use sails_rs::{gstd::msg, prelude::*};

pub struct VftProgram {
    authorities: RefCell<Authorities>,
    allowances: Pausable<RefCell<Allowances>>,
    balances: Pausable<RefCell<Balances>>,
    metadata: RefCell<Metadata>,
    pause: Pause,
}

#[program]
impl VftProgram {
    // Program's constructor
    pub fn new(name: String, symbol: String, decimals: u8) -> Self {
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
        let balances = Balances::default();

        let metadata = Metadata::new(name, symbol, decimals);

        Self {
            authorities: RefCell::new(Authorities::from_one(msg::source())),
            allowances: Pausable::new(&pause, RefCell::new(allowances)),
            balances: Pausable::new(&pause, RefCell::new(balances)),
            metadata: RefCell::new(metadata),
            pause,
        }
    }

    pub fn vft(&self) -> vft::Service<'_> {
        vft::Service::new(&self.allowances, &self.balances)
    }

    pub fn vft_admin(&self) -> vft_admin::Service<'_> {
        vft_admin::Service::new(
            &self.authorities,
            &self.allowances,
            &self.balances,
            &self.pause,
            self.vft(),
        )
    }

    pub fn vft_extension(&self) -> vft_extension::Service<'_> {
        vft_extension::Service::new(&self.allowances, &self.balances, self.vft())
    }

    pub fn vft_metadata(&self) -> vft_metadata::Service<'_> {
        vft_metadata::Service::new(&self.metadata)
    }
}
