use extended_vft_wasm::WASM_BINARY as TOKEN_WASM_BINARY;
use gtest::{Program, System, WasmProgram};
use sails_rs::prelude::*;
use vft_treasury_app::services::InitConfig;

pub const ADMIN_ID: u64 = 1000;
pub const TOKEN_ID: u64 = 200;
pub const ETH_CLIENT_ID: u64 = 500;

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
create_mock!(FTMockWrongReply, Ok(None));
create_mock!(
    FTMockReturnsFalse,
    Ok(Some(
        ["Vft".encode(), "Burn".encode(), false.encode()].concat()
    ))
);
create_mock!(
    FTMockReturnsTrue,
    Ok(Some(
        ["Vft".encode(), "Burn".encode(), true.encode()].concat()
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

pub trait Token {
    fn token(system: &System, id: u64) -> Program<'_>;
    fn approve(&self, from: u64, spender: ActorId, value: U256);
    fn mint(&self, from: u64, to: ActorId, value: U256);
    fn balance_of(&self, account: ActorId) -> U256;
}

impl Token for Program<'_> {
    fn token(system: &System, id: u64) -> Program<'_> {
        let token = Program::from_binary_with_id(system, id, TOKEN_WASM_BINARY);
        let payload = ["New".encode(), ("Token", "Token", 18).encode()].concat();
        let result = token.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        token
    }

    fn mint(&self, from: u64, to: ActorId, value: U256) {
        let payload = ["Vft".encode(), "Mint".encode(), (to, value).encode()].concat();
        assert!(!self.send_bytes(from, payload).main_failed());
    }

    fn approve(&self, from: u64, spender: ActorId, value: U256) {
        let payload = [
            "Vft".encode(),
            "Approve".encode(),
            (spender, value).encode(),
        ]
        .concat();

        assert!(!self.send_bytes(from, payload).main_failed());
    }

    fn balance_of(&self, account: ActorId) -> U256 {
        let query = ["Vft".encode(), "BalanceOf".encode(), account.encode()].concat();
        let result = self.send_bytes(ADMIN_ID, query.clone());

        let log_entry = result
            .log()
            .iter()
            .find(|log_entry| log_entry.destination() == ADMIN_ID.into())
            .expect("Unable to get query reply");

        let query_reply = <(String, String, U256)>::decode(&mut log_entry.payload())
            .expect("Unable to decode reply");
        query_reply.2
    }
}

pub trait VftTreasury {
    fn vft_treasury(system: &System) -> Program<'_>;
    fn deposit(&self, from: u64, token: ActorId, amount: U256);
    fn withdraw(&self, from: u64, token: ActorId, recepient: ActorId, amount: U256);
}

impl VftTreasury for Program<'_> {
    fn vft_treasury(system: &System) -> Program<'_> {
        let program = Program::current(system);
        let init_config = InitConfig {
            ethereum_event_client: ETH_CLIENT_ID.into(),
        };

        let payload = ["New".encode(), init_config.encode()].concat();
        let result = program.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        program
    }

    fn deposit(&self, from: u64, token: ActorId, amount: U256) {
        let payload = [
            "VftTreasury".encode(),
            "Deposit".encode(),
            (token, amount).encode(),
        ]
        .concat();

        let result = self.send_bytes(from, payload);
        assert!(!result.main_failed());
    }

    fn withdraw(&self, from: u64, token: ActorId, recepient: ActorId, amount: U256) {
        let payload = [
            "VftTreasury".encode(),
            "Withdraw".encode(),
            (token, recepient, amount).encode(),
        ]
        .concat();

        let result = self.send_bytes(from, payload);
        assert!(!result.main_failed());
    }
}
