#![no_std]

use sails_rs::{gstd::GStdExecContext, program};
pub mod services;
use services::{BridgingPayment, InitConfig};

/// Bridging Payment program.
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    /// Create Bridging Payment program.
    pub fn new(init_config: InitConfig) -> Self {
        BridgingPayment::<GStdExecContext>::seed(init_config);
        Self
    }

    /// Get Bridging Payment service.
    pub fn bridging_payment(&self) -> BridgingPayment<GStdExecContext> {
        BridgingPayment::new(GStdExecContext::new())
    }
}
