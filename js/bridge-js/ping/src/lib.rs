#![no_std]
use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Event {
    ReceiptSubmitted(u64, u32),
}

pub struct PingService;

#[service(events = Event)]
impl PingService {
    pub fn submit_receipt(&mut self, slot: u64, transaction_index: u32, _receipt_rlp: Vec<u8>) {
        self.emit_event(Event::ReceiptSubmitted(slot, transaction_index))
            .expect("Failed to emit event");
    }
}

pub struct PingProgram;

#[sails_rs::program]
impl PingProgram {
    pub fn new() -> Self {
        Self
    }
    pub fn ping(&self) -> PingService {
        PingService
    }
}
