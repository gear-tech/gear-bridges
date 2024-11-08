#![no_std]

use cell::RefCell;
use sails_rs::{
    gstd::{ExecContext, GStdExecContext},
    prelude::*,
};
use state::{Config, EndpointList};

pub mod error;
pub mod service;
pub mod state;

#[cfg(test)]
pub mod tests; 


pub struct HistoricalProxyProgram(RefCell<state::ProxyState>);

#[sails_rs::program]
impl HistoricalProxyProgram {
    // Program's constructor
    pub fn new(config: Config) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(state::ProxyState {
            admin: exec_context.actor_id(),
            endpoints: EndpointList::new(),
            config,
        }))
    }

    // Exposed service
    pub fn historical_proxy(&self) -> service::HistoricalProxyService<GStdExecContext> {
        service::HistoricalProxyService::new(&self.0, GStdExecContext::new())
    }
}
