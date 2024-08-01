#![no_std]

use sails_rs::{gstd::GStdExecContext, prelude::*};
pub mod services;
use services::{InitConfig, VftGateway};
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        VftGateway::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn vft_gateway(&self) -> VftGateway<GStdExecContext> {
        VftGateway::new(GStdExecContext::new())
    }
}
