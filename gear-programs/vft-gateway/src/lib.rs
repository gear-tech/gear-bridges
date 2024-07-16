#![no_std]

use sails::gstd::{gprogram, GStdExecContext};
pub mod services;
use services::{InitConfig, VftGateway};
#[derive(Default)]
pub struct Program;

#[gprogram(handle_reply = my_handle_reply)]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        VftGateway::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn vft_gateway(&self) -> VftGateway<GStdExecContext> {
        VftGateway::new(GStdExecContext::new())
    }
}

fn my_handle_reply() {
    VftGateway::<GStdExecContext>::handle_reply();
}
