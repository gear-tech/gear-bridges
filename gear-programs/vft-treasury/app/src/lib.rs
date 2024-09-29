#![no_std]

use sails_rs::{gstd::GStdExecContext, prelude::*};
pub mod services;
use services::{InitConfig, VftTreasury};
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        VftTreasury::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn vft_treasury(&self) -> VftTreasury<GStdExecContext> {
        VftTreasury::new(GStdExecContext::new())
    }
}
