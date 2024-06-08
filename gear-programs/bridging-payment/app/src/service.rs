use gear_programs_common::VARA2ETHRequest;
use sails_rtl::{gstd::gservice, prelude::*};

#[derive(Default)]
pub struct Service {
    fee: u128,
}

#[gservice]
impl Service {
    pub fn new(fee: u128) -> Self {
        Self { fee }
    }

    pub fn request_bridging(&mut self, request: VARA2ETHRequest) -> Result<String, String> {
        Ok("".into())
    }
}
