#![no_std]

use sails_rs::program;
pub mod services;
use services::{BridgingPayment, State};

/// Bridging Payment program.
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    /// Create Bridging Payment program.
    pub fn new(initial_state: State) -> Self {
        BridgingPayment::seed(initial_state);
        Self
    }

    /// Get Bridging Payment service.
    pub fn bridging_payment(&self) -> BridgingPayment {
        BridgingPayment::new()
    }
}
