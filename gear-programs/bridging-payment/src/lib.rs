#![no_std]

use sails::gstd::{gprogram, GStdExecContext};
pub mod services;
use services::{BridgePayment, InitConfig};
#[derive(Default)]
pub struct Program;

#[gprogram]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        BridgePayment::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn bridge_payment(&self) -> BridgePayment<GStdExecContext> {
        BridgePayment::new(GStdExecContext::new())
    }
}
