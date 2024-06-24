#![no_std]

use sails_rtl::gstd::{gprogram, GStdExecContext};
pub mod services;
use services::{GRC20Gateway, InitConfig};
#[derive(Default)]
pub struct Program;

#[gprogram(handle_reply = my_handle_reply)]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        GRC20Gateway::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn grc20_gateway(&self) -> GRC20Gateway<GStdExecContext> {
        GRC20Gateway::new(GStdExecContext::new())
    }
}

fn my_handle_reply() {
    GRC20Gateway::<GStdExecContext>::handle_reply();
}
