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
    /// `mocks` is enabled.
    pub fn gas_calculation(_init_config: InitConfig, _slot_first: u64, _count: Option<u32>) -> Self {
        #[cfg(feature = "mocks")]
        {
            let self_ = Self::new(_init_config);

            let transactions = services::submit_receipt::transactions_mut();
            let count = _count.map(|c| c as usize).unwrap_or(services::SIZE_FILL_TRANSACTIONS_STEP);
            for i in 0..count {
                transactions.insert((_slot_first, i as u64));
            }

            self_
        }

        #[cfg(not(feature = "mocks"))]
        panic!("Please rebuild with enabled `mocks` feature")
    }

    pub fn vft_manager(&self) -> VftManager<GStdExecContext> {
        VftManager::new(GStdExecContext::new())
    }
}
