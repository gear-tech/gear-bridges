use bridging_payment_vara_supply::services::{Config, InitConfig};
use extended_vft_wasm::WASM_BINARY as TOKEN_WASM_BINARY;
use gtest::{Program, System, WasmProgram};
use sails_rs::prelude::*;
use vft_treasury_wasm::WASM_BINARY as VFT_TREASURY_WASM_BINARY;

pub const ADMIN_ID: u64 = 1000;
pub const FEE: u128 = 10_000_000_000_000;
pub const VFT_TREASURY_ID: u64 = 100;
pub const TOKEN_ID: u64 = 200;
pub const ETH_CLIENT_ID: u64 = 500;
pub const BRIDGE_BUILTIN_ID: u64 = 300;

// macros
macro_rules! create_function {
    ($name:ident, $method:expr) => {
        fn $name(&self, from: u64, error: bool);
    };

    ($name:ident, $method:expr, $($param_name:ident: $param_type:ty),*) => {
        fn $name(&self, from: u64, $($param_name: $param_type,)* error: bool);
    };
}

macro_rules! implement_function {
    ($name:ident, $prefix:expr, $method:expr) => {
        fn $name(&self, from: u64, error: bool) {
            let payload = [
                $prefix.encode(),
                $method.encode(),
            ]
            .concat();
            let result = self.send_bytes(from, payload);

            if error {
                assert!(result.main_failed());
            } else {
                assert!(!result.main_failed());
            }
        }
    };
    ($name:ident, $prefix:expr, $method:expr, $($param_name:ident: $param_type:ty),*; $with_value:expr) => {
        fn $name(&self, from: u64, $($param_name: $param_type,)* error: bool) {
            let payload = [
                $prefix.encode(),
                $method.encode(),
                ($($param_name,)*).encode(),
            ]
            .concat();

            let result = if $with_value {
                self.send_bytes_with_value(from, payload, FEE)
            } else {
                self.send_bytes(from, payload)
            };

            if error {
                assert!(result.main_failed());
            } else {
                assert!(!result.main_failed());
            }
        }
    };
}

macro_rules! create_query_function {
    // Match functions with parameters
    ($fn_name:ident, $return_type:ty, $($param_name:ident: $param_type:ty),*) => {
        fn $fn_name(&self, $($param_name: $param_type),*) -> $return_type;
    };
    // Match functions without parameters
    ($fn_name:ident, $return_type:ty) => {
        fn $fn_name(&self) -> $return_type;
    };
}

macro_rules! implement_token_query {
    ($fn_name:ident, $query_name:expr, $return_type:ty) => {
        fn $fn_name(&self) -> $return_type {
            let query = ["Vft".encode(), $query_name.encode()].concat();
            let result = self.send_bytes(ADMIN_ID, query.clone());

            let log_entry = result
                .log()
                .iter()
                .find(|log_entry| log_entry.destination() == ADMIN_ID.into())
                .expect("Unable to get query reply");

            let query_reply = <(String, String, $return_type)>::decode(&mut log_entry.payload())
                .expect("Unable to decode reply");
            query_reply.2
        }
    };

    ($fn_name:ident, $query_name:expr, $return_type:ty, $($param_name:ident: $param_type:ty),*) => {
        fn $fn_name(&self, $($param_name: $param_type),*) -> $return_type {
            let query = ["Vft".encode(), $query_name.encode(), ($($param_name),*).encode()].concat();
            let result = self.send_bytes(ADMIN_ID, query.clone());

            let log_entry = result
                .log()
                .iter()
                .find(|log_entry| log_entry.destination() == ADMIN_ID.into())
                .expect("Unable to get query reply");

            let query_reply = <(String, String, $return_type)>::decode(&mut log_entry.payload())
                .expect("Unable to decode reply");
            query_reply.2
        }
    };
}

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

// Smart contract functionality for Program
pub trait BridgingPayment {
    fn bridge_payment(system: &System) -> Program<'_>;
    create_function!(request, "Request", amount: U256, receiver: H160, vara_token_id: ActorId);
    create_function!(reclaim_fee, "ReclaimFee");
}

impl BridgingPayment for Program<'_> {
    fn bridge_payment(system: &System) -> Program<'_> {
        let program = Program::current(system);
        let init_config = InitConfig::new(
            ADMIN_ID.into(),
            VFT_TREASURY_ID.into(),
            Config::new(FEE, 15_000_000_000, 100_000_000_000, 1000, 50_000_000_000),
        );
        let payload = ["New".encode(), init_config.encode()].concat();
        let result = program.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        program
    }
    implement_function!(request, "BridgingPayment", "Request", amount: U256, receiver: H160, vara_token_id: ActorId; true);
    implement_function!(reclaim_fee, "BridgingPayment", "ReclaimFee");
}

pub trait Token {
    fn token(system: &System, id: u64) -> Program<'_>;
    create_function!(mint, "Mint", to: ActorId, value: U256);
    create_function!(approve, "Approve", spender: ActorId, value: U256);
    create_query_function!(balance_of, U256, account: ActorId);
}

impl Token for Program<'_> {
    fn token(system: &System, id: u64) -> Program<'_> {
        let token = Program::from_binary_with_id(system, id, TOKEN_WASM_BINARY);
        let payload = ["New".encode(), ("Token", "Token", 18).encode()].concat();
        let result = token.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        token
    }
    implement_function!(mint, "Vft", "Mint", to: ActorId, value: U256; false);
    implement_function!(approve,"Vft", "Approve", spender: ActorId, value: U256; false);
    implement_token_query!(balance_of, "BalanceOf", U256, account: ActorId);
}

pub trait VftTreasury {
    fn vft_treasury(system: &System) -> Program<'_>;
    create_function!(map_vara_to_eth_address, "MapVaraToEthAddress", eth_token_id: H160, vara_token_id: ActorId);
}

impl VftTreasury for Program<'_> {
    fn vft_treasury(system: &System) -> Program<'_> {
        let program =
            Program::from_binary_with_id(system, VFT_TREASURY_ID, VFT_TREASURY_WASM_BINARY);
        let init_config = vft_treasury_app::services::InitConfig::new(
            [1; 20].into(),
            BRIDGE_BUILTIN_ID.into(),
            ETH_CLIENT_ID.into(),
            vft_treasury_app::services::Config::new(
                15_000_000_000,
                15_000_000_000,
                15_000_000_000,
                100,
                15_000_000_000,
            ),
        );

        let payload = ["New".encode(), init_config.encode()].concat();
        let result = program.send_bytes(ADMIN_ID, payload);
        assert!(!result.main_failed());
        program
    }
    implement_function!(map_vara_to_eth_address, "VftTreasury", "MapVaraToEthAddress", eth_token_id: H160, vara_token_id: ActorId; false);
}