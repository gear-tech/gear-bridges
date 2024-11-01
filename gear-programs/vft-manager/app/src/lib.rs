#![no_std]

use sails_rs::{gstd::GStdExecContext, prelude::*};
pub mod services;
use services::{InitConfig, VftManager};
#[derive(Default)]
pub struct Program;

#[program]
impl Program {
    pub fn new(init_config: InitConfig) -> Self {
        VftManager::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    pub fn vft_manager(&self) -> VftManager<GStdExecContext> {
        VftManager::new(GStdExecContext::new())
    }
}
