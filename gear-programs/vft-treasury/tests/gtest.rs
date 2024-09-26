use gtest::{Program, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_treasury_app::services::vft::traits::*;
use vft_treasury_app::services::vft::{Vft as VftC, VftFactory as VftFactoryC};
use vft_treasury_client::{
    traits::*, Config, InitConfig, VftTreasury as VftTreasuryC,
    VtfTreasuryFactory as VftTreasuryFactoryC, Error
};

use extended_vft_wasm::WASM_BINARY as TOKEN_WASM_BINARY;

pub const ADMIN_ID: u64 = 1000;
pub const TOKEN_ID: u64 = 200;
pub const ETH_CLIENT_ID: u64 = 500;
pub const BRIDGE_BUILTIN_ID: u64 = 300;

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum Response {
    MessageSent { nonce: U256, hash: H256 },
}

#[derive(Debug)]
struct GearBridgeBuiltinMock;

impl WasmProgram for GearBridgeBuiltinMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(Some(
            Response::MessageSent {
                nonce: U256::from(1),
                hash: [1; 32].into(),
            }
            .encode(),
        ))
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

struct Fixture {
    remoting: GTestRemoting,
    treasury_program_id: ActorId,
    vft_program_id: ActorId,
}

async fn setup_for_test() -> Fixture {
    let remoting = GTestRemoting::new(ADMIN_ID.into());
    remoting.system().init_logger();

    // Bridge Builtin
    let gear_bridge_builtin =
        Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    let _ = gear_bridge_builtin.send_bytes(ADMIN_ID, b"INIT");

    // Treasury
    let treasury_code_id = remoting.system().submit_code(vft_treasury::WASM_BINARY);
    let init_config = InitConfig {
        receiver_contract_address: [1; 20].into(),
        gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
        ethereum_event_client: ETH_CLIENT_ID.into(),
        config: Config {
            gas_for_transfer_tokens: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
            gas_for_transfer_to_eth_msg: 15_000_000_000,
        },
    };
    let treasury_program_id = VftTreasuryFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(treasury_code_id, b"salt")
        .await
        .unwrap();

    // VFT
    let vft_code_id = remoting.system().submit_code(TOKEN_WASM_BINARY);
    let vft_program_id = VftFactoryC::new(remoting.clone())
        .new("Token".into(), "Token".into(), 18)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    Fixture {
        remoting,
        treasury_program_id,
        vft_program_id,
    }
}

async fn balance_of(
    remoting: &GTestRemoting,
    vtf_program_id: ActorId,
    program_id: ActorId,
) -> U256 {
    VftC::new(remoting.clone())
        .balance_of(program_id)
        .recv(vtf_program_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_treasury() {
    let Fixture {
        remoting,
        treasury_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let mut vft_treasury = VftTreasuryC::new(remoting.clone());
    vft_treasury
        .map_vara_to_eth_address([2; 20].into(), vft_program_id)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_id: ActorId = 100000.into();
    let amount = U256::from(10_000_000_000_u64);

    let mut vft = VftC::new(remoting.clone());

    let ok = vft
        .mint(account_id.into(), amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let reply = vft_treasury
        .deposit_tokens(vft_program_id, account_id, amount, [3; 20].into())
        .send_recv(treasury_program_id)
        .await
        .unwrap();

    let expected = Ok((U256::from(1), H160::from([2; 20])));
    assert_eq!(reply, expected);

    let account_balance = balance_of(&remoting, vft_program_id, account_id).await;
    assert!(account_balance.is_zero());

    let treasury_balance = balance_of(&remoting, vft_program_id, treasury_program_id).await;
    assert_eq!(treasury_balance, amount);

    VftTreasuryC::new(remoting.clone().with_actor_id(ETH_CLIENT_ID.into()))
        .withdraw_tokens([2; 20].into(), account_id, amount)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_balance = balance_of(&remoting, vft_program_id, account_id).await;
    assert_eq!(account_balance, amount);

    let treasury_balance = balance_of(&remoting, vft_program_id, treasury_program_id).await;
    assert!(treasury_balance.is_zero());
}

#[tokio::test]
async fn test_mapping_does_not_exists() {
    let Fixture {
        remoting,
        treasury_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id: ActorId = 100000.into();
    let amount = U256::from(10_000_000_000_u64);

    let ok = VftC::new(remoting.clone())
        .mint(account_id.into(), amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let reply = VftTreasuryC::new(remoting.clone())
        .deposit_tokens(vft_program_id, account_id, amount, [3; 20].into())
        .send_recv(treasury_program_id)
        .await
        .unwrap();

    assert!(reply.is_err());
    assert_eq!(reply.unwrap_err(), Error::NoCorrespondingEthAddress);
}

#[tokio::test]
async fn test_withdraw_fails_with_bad_origin() {
    let Fixture {
        remoting,
        treasury_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let mut vft_treasury = VftTreasuryC::new(remoting.clone());
    vft_treasury
        .map_vara_to_eth_address([2; 20].into(), vft_program_id)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_id: ActorId = 100000.into();

    let result = vft_treasury.withdraw_tokens(
        [2; 20].into(),
        account_id,
        U256::from(42),
    )
        .send_recv(treasury_program_id)
        .await
        .unwrap();

    assert_eq!(result.unwrap_err(), Error::NotEthClient);
}

#[tokio::test]
async fn test_anyone_can_deposit() {
    let Fixture {
        remoting,
        treasury_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let mut vft_treasury = VftTreasuryC::new(remoting.clone());
    vft_treasury
        .map_vara_to_eth_address([2; 20].into(), vft_program_id)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .unwrap();

    let account0_id: ActorId = 100000.into();
    let account1_id: ActorId = 100001.into();
    let amount = U256::from(10_000_000_000_u64);

    let mut vft = VftC::new(remoting.clone());

    let ok = vft
        .mint(account0_id.into(), amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = vft
        .mint(account1_id.into(), amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account0_id))
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account1_id))
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let reply = VftTreasuryC::new(remoting.clone().with_actor_id(account1_id))
        .deposit_tokens(vft_program_id, account0_id, amount, [3; 20].into())
        .send_recv(treasury_program_id)
        .await
        .unwrap();

    let expected = Ok((U256::from(1), H160::from([2; 20])));

    assert_eq!(reply, expected);
    let account0_balance = balance_of(&remoting, vft_program_id, account0_id).await;
    assert!(account0_balance.is_zero());
    let treasury_balance = balance_of(&remoting, vft_program_id, treasury_program_id).await;
    assert_eq!(treasury_balance, amount);

    let reply = VftTreasuryC::new(remoting.clone().with_actor_id(account0_id))
        .deposit_tokens(vft_program_id, account1_id, amount, [3; 20].into())
        .send_recv(treasury_program_id)
        .await
        .unwrap();

    assert_eq!(reply, expected);
    let account1_balance = balance_of(&remoting, vft_program_id, account1_id).await;
    assert!(account1_balance.is_zero());
    let treasury_balance = balance_of(&remoting, vft_program_id, treasury_program_id).await;
    assert_eq!(treasury_balance, amount*2);
}
