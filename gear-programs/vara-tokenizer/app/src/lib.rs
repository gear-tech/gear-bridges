#![no_std]

#[macro_use]
mod storage;

mod admin_service;
mod tokenizer_service;
mod vft_funcs;

use admin_service::{AdminConfig, AdminService};
use sails_rs::{collections::HashSet, gstd::msg, prelude::*};
use tokenizer_service::TokenizerService;

pub struct VaraTokenizerProgram(());

#[sails_rs::program]
impl VaraTokenizerProgram {
    // Program's constructor
    pub fn new(name: String, symbol: String, decimals: u8, set_admin: bool) -> Self {
        vft_service::Service::seed(name, symbol, decimals);
        let admins: HashSet<ActorId> = if set_admin {
            [msg::source()].into()
        } else {
            [].into()
        };
        admin_service::init(AdminConfig { admins });
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

    // Exposed admin service
    pub fn admin(&self) -> AdminService {
        AdminService::new()
    }
}
