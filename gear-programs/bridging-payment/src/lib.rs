#![no_std]

use sails_rs::{gstd::GStdExecContext, program};
pub mod services;
use services::{BridgePayment, InitConfig};
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        BridgePayment::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn bridge_payment(&self) -> BridgePayment<GStdExecContext> {
        BridgePayment::new(GStdExecContext::new())
    }
}
