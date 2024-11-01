#![no_std]
#![allow(clippy::new_without_default)]
use core::fmt::Debug;
use gstd::msg;
use sails_rs::{collections::HashMap, gstd::service, prelude::*};

pub mod funcs;
pub mod utils;

static mut STORAGE: Option<Storage> = None;

#[derive(Debug, Default)]
pub struct Storage {
    balances: HashMap<ActorId, U256>,
    allowances: HashMap<(ActorId, ActorId), U256>,
    meta: Metadata,
    total_supply: U256,
}

impl Storage {
    pub fn get_mut() -> &'static mut Self {
        unsafe { STORAGE.as_mut().expect("Storage is not initialized") }
    }
    pub fn get() -> &'static Self {
        unsafe { STORAGE.as_ref().expect("Storage is not initialized") }
    }
    pub fn balances() -> &'static mut HashMap<ActorId, U256> {
        let storage = unsafe { STORAGE.as_mut().expect("Storage is not initialized") };
        &mut storage.balances
    }
    pub fn total_supply() -> &'static mut U256 {
        let storage = unsafe { STORAGE.as_mut().expect("Storage is not initialized") };
        &mut storage.total_supply
    }
}

#[derive(Debug, Default)]
pub struct Metadata {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Event {
    Approval {
        owner: ActorId,
        spender: ActorId,
        value: U256,
    },
    Transfer {
        from: ActorId,
        to: ActorId,
        value: U256,
    },
}

#[derive(Clone)]
pub struct Service();

impl Service {
    pub fn seed(name: String, symbol: String, decimals: u8) -> Self {
        unsafe {
            STORAGE = Some(Storage {
                meta: Metadata {
                    name,
                    symbol,
                    decimals,
                },
                ..Default::default()
            });
        }
        Self()
    }
}

#[service(events = Event)]
impl Service {
    pub fn new() -> Self {
        Self()
    }

    pub fn approve(&mut self, spender: ActorId, value: U256) -> bool {
        let owner = msg::source();
        let storage = Storage::get_mut();
        let mutated = funcs::approve(&mut storage.allowances, owner, spender, value);

        if mutated {
            self.notify_on(Event::Approval {
                owner,
                spender,
                value,
            })
            .expect("Notification Error");
        }

        mutated
    }

    pub fn transfer(&mut self, to: ActorId, value: U256) -> bool {
        let from = msg::source();
        let storage = Storage::get_mut();
        let mutated =
            utils::panicking(move || funcs::transfer(&mut storage.balances, from, to, value));

        if mutated {
            self.notify_on(Event::Transfer { from, to, value })
                .expect("Notification Error");
        }

        mutated
    }

    pub fn transfer_from(&mut self, from: ActorId, to: ActorId, value: U256) -> bool {
        let spender = msg::source();
        let storage = Storage::get_mut();
        let mutated = utils::panicking(move || {
            funcs::transfer_from(
                &mut storage.allowances,
                &mut storage.balances,
                spender,
                from,
                to,
                value,
            )
        });

        if mutated {
            self.notify_on(Event::Transfer { from, to, value })
                .expect("Notification Error");
        }

        mutated
    }

    pub fn allowance(&self, owner: ActorId, spender: ActorId) -> U256 {
        let storage = Storage::get();
        funcs::allowance(&storage.allowances, owner, spender)
    }

    pub fn balance_of(&self, account: ActorId) -> U256 {
        let storage = Storage::get();
        funcs::balance_of(&storage.balances, account)
    }

    pub fn decimals(&self) -> &'static u8 {
        let storage = Storage::get();
        &storage.meta.decimals
    }

    pub fn name(&self) -> &'static str {
        let storage = Storage::get();
        &storage.meta.name
    }

    pub fn symbol(&self) -> &'static str {
        let storage = Storage::get();
        &storage.meta.symbol
    }

    pub fn total_supply(&self) -> &'static U256 {
        let storage = Storage::get();
        &storage.total_supply
    }
}
