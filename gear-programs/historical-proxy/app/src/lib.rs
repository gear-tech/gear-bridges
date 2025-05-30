#![no_std]

use cell::RefCell;
use sails_rs::prelude::*;
use state::EndpointList;

pub mod error;
pub mod service;
pub mod state;

#[cfg(test)]
pub mod tests;

pub struct HistoricalProxyProgram(RefCell<state::ProxyState>);

#[sails_rs::program]
impl HistoricalProxyProgram {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(RefCell::new(state::ProxyState {
            admin: Syscall::message_source(),
            endpoints: EndpointList::new(),
        }))
    }

    pub fn historical_proxy(&self) -> service::HistoricalProxyService {
        service::HistoricalProxyService::new(&self.0)
    }
}
