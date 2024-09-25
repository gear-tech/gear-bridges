#![no_std]

use sails_rs::{gstd::GStdExecContext, program};
pub mod services;
use services::{BridgingPayment, InitConfig};
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        BridgingPayment::<GStdExecContext>::seed(init_config);
        Self
    }

    pub fn bridging_payment(&self) -> BridgingPayment<GStdExecContext> {
        BridgingPayment::new(GStdExecContext::new())
    }
}
