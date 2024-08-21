use extended_vft_wasm::WASM_BINARY as TOKEN_WASM_BINARY;
use gtest::{Program, System, WasmProgram};
use vft_gateway_app::services::{error::Error, msg_tracker::MessageInfo, Config, InitConfig};

use sails_rs::prelude::*;
pub const ADMIN_ID: u64 = 1000;
pub const TOKEN_ID: u64 = 200;
pub const BRIDGE_BUILTIN_ID: u64 = 300;

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
    fn mint(&self, from: u64, to: ActorId, value: U256);
    fn grant_burner_role(&self, from: u64, to: ActorId);
    fn grant_minter_role(&self, from: u64, to: ActorId);
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

    fn grant_burner_role(&self, from: u64, to: ActorId) {
        let payload = ["Vft".encode(), "GrantBurnerRole".encode(), to.encode()].concat();
        assert!(!self.send_bytes(from, payload).main_failed());
    }

    fn grant_minter_role(&self, from: u64, to: ActorId) {
        let payload = ["Vft".encode(), "GrantMinterRole".encode(), to.encode()].concat();
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

pub trait VftGateway {
    fn vft_gateway(system: &System) -> Program<'_>;
    fn map_vara_to_eth_address(&self, from: u64, vara_token_id: ActorId, eth_token_id: H160);
    fn transfer_vara_to_eth(
        &self,
        from: u64,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
        with_gas: u64,
        panic: bool,
    ) -> Result<(U256, H160), Error>;
    fn handle_interrupted_transfer(
        &self,
        from: u64,
        msg_id: MessageId,
    ) -> Result<(U256, H160), Error>;
    fn get_msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)>;
}

impl VftGateway for Program<'_> {
    fn vft_gateway(system: &System) -> Program<'_> {
        let program = Program::current(system);
        let init_config = InitConfig::new(
            [1; 20].into(),
            BRIDGE_BUILTIN_ID.into(),
            Config::new(
                2_000_000_000,
                15_000_000_000,
                15_000_000_000,
                15_000_000_000,
                100,
                20_000_000_000,
            ),
        );
        let payload = ["New".encode(), init_config.encode()].concat();
        let result = program.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        program
    }

    fn map_vara_to_eth_address(&self, from: u64, vara_token_id: ActorId, eth_token_id: H160) {
        let payload = [
            "VftGateway".encode(),
            "MapVaraToEthAddress".encode(),
            (vara_token_id, eth_token_id).encode(),
        ]
        .concat();

        let result = self.send_bytes(from, payload);

        assert!(!result.main_failed());
    }

    fn transfer_vara_to_eth(
        &self,
        from: u64,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
        with_gas: u64,
        panic: bool,
    ) -> Result<(U256, H160), Error> {
        let payload = [
            "VftGateway".encode(),
            "TransferVaraToEth".encode(),
            (vara_token_id, amount, receiver).encode(),
        ]
        .concat();

        let result = self.send_bytes_with_gas(from, payload, with_gas, 0);
        if panic {
            assert!(result.main_failed());
            return Err(Error::MessageFailed);
        } else {
            let log_entry = result
                .log()
                .iter()
                .find(|log_entry| log_entry.destination() == from.into())
                .expect("Unable to get reply");

            let reply =
                <(String, String, Result<(U256, H160), Error>)>::decode(&mut log_entry.payload())
                    .expect("Unable to decode reply"); // Panic if decoding fails

            return reply.2;
        }
    }

    fn handle_interrupted_transfer(
        &self,
        from: u64,
        msg_id: MessageId,
    ) -> Result<(U256, H160), Error> {
        let payload = [
            "VftGateway".encode(),
            "HandleInterruptedTransfer".encode(),
            msg_id.encode(),
        ]
        .concat();
        let result = self.send_bytes(from, payload);
        let log_entry = result
            .log()
            .iter()
            .find(|log_entry| log_entry.destination() == from.into())
            .expect("Unable to get reply");

        let reply =
            <(String, String, Result<(U256, H160), Error>)>::decode(&mut log_entry.payload())
                .expect("Unable to decode reply"); // Panic if decoding fails

        return reply.2;
    }

    fn get_msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)> {
        let payload = ["VftGateway".encode(), "MsgTrackerState".encode()].concat();

        let result = self.send_bytes(ADMIN_ID, payload);
        let log_entry = result
            .log()
            .iter()
            .find(|log_entry| log_entry.destination() == ADMIN_ID.into())
            .expect("Unable to get reply");

        let reply =
            <(String, String, Vec<(MessageId, MessageInfo)>)>::decode(&mut log_entry.payload())
                .expect("Unable to decode reply"); // Panic if decoding fails

        return reply.2;
    }
}
