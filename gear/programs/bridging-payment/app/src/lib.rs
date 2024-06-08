#![no_std]

use sails_rtl::gstd::gprogram;
use service::Service;

pub mod service;

#[derive(Default)]
pub struct Program;

#[gprogram]
impl Program {
    pub fn new() -> Self {
        Self
    }

    pub fn ping(&self) -> service::Service {
        Service::new(10)
    }
}
