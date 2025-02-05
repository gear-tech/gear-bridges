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

    /// The constructor is intended for test purposes and is available only when the feature
    /// `gas_calculation` is enabled.
    pub fn gas_calculation(_init_config: InitConfig, _slot_first: u64) -> Self {
        #[cfg(feature = "gas_calculation")]
        {
            let self_ = Self::new(_init_config);

            let transactions = services::submit_receipt::transactions_mut();
            for i in 0..services::SIZE_FILL_TRANSACTIONS_STEP {
                transactions.insert((_slot_first, i as u64));
            }

            self_
        }

        #[cfg(not(feature = "gas_calculation"))]
        panic!("Please rebuild with enabled `gas_calculation` feature")
    }

    pub fn vft_manager(&self) -> VftManager<GStdExecContext> {
        VftManager::new(GStdExecContext::new())
    }
}
