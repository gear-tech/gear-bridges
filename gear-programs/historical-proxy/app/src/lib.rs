#![no_std]

use sails_rs::prelude::*;

pub mod service;

pub struct HistoricalProxyProgram(());

#[sails_rs::program]
impl HistoricalProxyProgram {
    // Program's constructor
    pub fn new() -> Self {
        Self(())
    }

    // Exposed service
    pub fn historical_proxy(&self) -> service::HistoricalProxyService {
        service::HistoricalProxyService::new()
    }
}
