#![no_std]

mod admin_service;
mod extended_vft_client;
mod tokenizer_service;

use admin_service::AdminService;
use sails_rs::{
    gstd::{calls::GStdRemoting, msg},
    prelude::*,
};
use tokenizer_service::TokenizerService;

pub struct VaraTokenizerProgram(());

#[sails_rs::program]
impl VaraTokenizerProgram {
    // Program's constructor
    pub fn new(vft_program_id: ActorId) -> Self {
        tokenizer_service::init(vft_program_id);
        admin_service::init(msg::source());
        Self(())
    }

    // Exposed tokenizer service
    pub fn tokenizer(&self) -> TokenizerService<crate::extended_vft_client::Vft<GStdRemoting>> {
        let vft_client = crate::extended_vft_client::Vft::new(GStdRemoting);
        TokenizerService::new(vft_client)
    }

    // Exposed admin service
    pub fn admin(&self) -> AdminService {
        AdminService::new()
    }
}
