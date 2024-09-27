use gtest::WasmProgram;
use sails_rs::prelude::*;

// Mocks for programs
macro_rules! create_mock {
    ($name:ident, $handle_result:expr) => {
        #[derive(Debug)]
        pub struct $name;

        impl WasmProgram for $name {
            fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
                Ok(None)
            }

            fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
                $handle_result
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
    };
}

create_mock!(FTMockError, Err("Error"));
create_mock!(
    FTMockReturnsFalse,
    Ok(Some(
        ["Vft".encode(), "Burn".encode(), false.encode()].concat()
    ))
);

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum Response {
    MessageSent { nonce: U256, hash: H256 },
}

create_mock!(
    GearBridgeBuiltinMock,
    Ok(Some(
        Response::MessageSent {
            nonce: U256::from(1),
            hash: [1; 32].into(),
        }
        .encode(),
    ))
);
create_mock!(GearBridgeBuiltinMockPanic, Err("Error"));
