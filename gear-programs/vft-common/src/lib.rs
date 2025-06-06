#![no_std]

use awesome_sails::{
    error::Error,
    pause::Pausable,
    storage::{InfallibleStorage, Storage},
};
use awesome_sails_services::{
    vft::{
        self,
        utils::{Allowances, Balances},
    },
    vft_metadata::{self, Metadata},
};
use core::cell::RefCell;
use sails_rs::prelude::*;

pub struct Service<
    'a,
    A = Pausable<RefCell<Allowances>>,
    B = Pausable<RefCell<Balances>>,
    M = RefCell<Metadata>,
> {
    // Allowances storage.
    allowances: &'a A,
    // Balances storage.
    balances: &'a B,
    // Metadata storage.
    metadata: &'a M,
}

impl<'a, A, B, M> Service<'a, A, B, M> {
    pub fn new(allowances: &'a A, balances: &'a B, metadata: &'a M) -> Self {
        Self {
            allowances,
            balances,
            metadata,
        }
    }

    fn vft(&self) -> vft::Service<'_, A, B> {
        vft::Service::new(self.allowances, self.balances)
    }

    fn metadata(&self) -> vft_metadata::Service<'_, M> {
        vft_metadata::Service::new(self.metadata)
    }
}

#[service(events = vft::Event)]
impl<
        A: Storage<Item = Allowances>,
        B: Storage<Item = Balances>,
        M: InfallibleStorage<Item = Metadata>,
    > Service<'_, A, B, M>
{
    pub fn name(&self) -> String {
        self.metadata().name()
    }

    /// Returns the symbol of the VFT.
    pub fn symbol(&self) -> String {
        self.metadata().symbol()
    }

    /// Returns the number of decimals of the VFT.
    pub fn decimals(&self) -> u8 {
        self.metadata().decimals()
    }

    #[export(unwrap_result)]
    pub fn approve(&mut self, spender: ActorId, value: U256) -> Result<bool, Error> {
        self.vft().approve(spender, value)
    }

    #[export(unwrap_result)]
    pub fn transfer(&mut self, to: ActorId, value: U256) -> Result<bool, Error> {
        self.vft().transfer(to, value)
    }

    #[export(unwrap_result)]
    pub fn transfer_from(
        &mut self,
        from: ActorId,
        to: ActorId,
        value: U256,
    ) -> Result<bool, Error> {
        self.vft().transfer_from(from, to, value)
    }

    #[export(unwrap_result)]
    pub fn allowance(&self, owner: ActorId, spender: ActorId) -> Result<U256, Error> {
        self.vft().allowance(owner, spender)
    }

    #[export(unwrap_result)]
    pub fn balance_of(&self, account: ActorId) -> Result<U256, Error> {
        self.vft().balance_of(account)
    }

    #[export(unwrap_result)]
    pub fn total_supply(&self) -> Result<U256, Error> {
        self.vft().total_supply()
    }
}
