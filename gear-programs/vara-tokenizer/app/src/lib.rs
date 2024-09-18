#![no_std]

mod tokenizer_service;
mod vft_funcs;

use sails_rs::prelude::*;
use tokenizer_service::TokenizerService;

pub struct VaraTokenizerProgram(());

#[sails_rs::program]
impl VaraTokenizerProgram {
    // Program's constructor
    pub fn new(name: String, symbol: String, decimals: u8) -> Self {
        vft_service::Service::seed(name, symbol, decimals);
        Self(())
    }

    // Exposed tokenizer service
    pub fn tokenizer(&self) -> TokenizerService {
        TokenizerService::new()
    }

    // Exposed vft service
    pub fn vft(&self) -> vft_service::Service {
        vft_service::Service::new()
    }
}
