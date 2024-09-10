use gtest::WasmProgram;
use sails_rs::{
    calls::ActionIo, gtest::calls::GTestRemoting, ActorId, CodeId, Decode, Encode, U256,
};

pub const ADMIN_ID: u64 = 42;
pub const VFT_PROGRAM_ID: u64 = 300;

pub fn init_remoting() -> (GTestRemoting, CodeId) {
    let remoting = GTestRemoting::new(ADMIN_ID.into());
    remoting.system().mint_to(ADMIN_ID, 100_000_000_000_000);
    remoting.system().init_logger();

    // Submit program code into the system
    let program_code_id = remoting.system().submit_code(vara_tokenizer::WASM_BINARY);
    (remoting, program_code_id)
}

// Mocks for programs
#[derive(Debug)]
pub struct ExtendedVftMock;

impl WasmProgram for ExtendedVftMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        if payload.starts_with(vft_client::vft::io::Mint::ROUTE) {
            let mut input = &payload[vft_client::vft::io::Mint::ROUTE.len()..];
            let (_to, _value): (ActorId, U256) = Decode::decode(&mut input).unwrap();
            let reply = [vft_client::vft::io::Mint::ROUTE, true.encode().as_slice()].concat();
            return Ok(Some(reply));
        }
        if payload.starts_with(vft_client::vft::io::Burn::ROUTE) {
            let reply = [vft_client::vft::io::Burn::ROUTE, true.encode().as_slice()].concat();
            return Ok(Some(reply));
        }
        Ok(None)
    }

    fn handle_reply(&mut self, _payload: Vec<u8>) -> Result<(), &'static str> {
        unimplemented!()
    }

    fn handle_signal(&mut self, _payload: Vec<u8>) -> Result<(), &'static str> {
        unimplemented!()
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        unimplemented!()
    }
}
