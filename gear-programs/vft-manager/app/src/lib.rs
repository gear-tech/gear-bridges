#![no_std]

use sails_rs::{gstd::GStdExecContext, prelude::*};
pub mod services;
use services::{InitConfig, VftManager};

/// VFT Manager program.
#[derive(Default)]
pub struct Program;

/// VFT Manager program implementation.
#[program]
impl Program {
    /// Create VFT Manager program.
    pub fn new(init_config: InitConfig) -> Self {
        VftManager::<GStdExecContext>::seed(init_config, GStdExecContext::new());
        Self
    }

    /// Get VFT Manager service.
    pub fn vft_manager(&self) -> VftManager<GStdExecContext> {
        VftManager::new(GStdExecContext::new())
    }
}
