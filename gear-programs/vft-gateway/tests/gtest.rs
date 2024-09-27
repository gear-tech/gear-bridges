use blake2::{digest::typenum::U32, Blake2b, Digest};
use gtest::Program;
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_gateway_app::services::vft::{traits::*, Vft as VftC, VftFactory as VftFactoryC};
use vft_gateway_client::{
    traits::*, Config, Error, InitConfig, MessageStatus, VftGateway as VftGatewayC,
    VftGatewayFactory as VftGatewayFactoryC,
};

mod utils;
use utils::*;

const ADMIN_ID: u64 = 1000;
const TOKEN_ID: u64 = 200;
const ETH_CLIENT_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;

#[derive(Default)]
enum BridgeBuiltinMock {
    #[default]
    GearBridgeBuiltinMock,
    GearBridgeBuiltinMockPanic,
}

#[derive(Default)]
enum VftMock {
    #[default]
    Binary,
    FTMockError,
    FTMockReturnsFalse,
}

struct Fixture {
    remoting: GTestRemoting,
    gateway_program_id: ActorId,
    vft_program_id: ActorId,
}

async fn setup_for_test() -> Fixture {
    setup_for_test_with_mocks(Default::default(), Default::default()).await
}

async fn setup_for_test_with_mocks(
    bridge_builtin_mock: BridgeBuiltinMock,
    vft_mock: VftMock,
) -> Fixture {
    let remoting = GTestRemoting::new(ADMIN_ID.into());
    remoting.system().init_logger();

    // Bridge Builtin
    let gear_bridge_builtin = match bridge_builtin_mock {
        BridgeBuiltinMock::GearBridgeBuiltinMock => {
            Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock)
        }
        BridgeBuiltinMock::GearBridgeBuiltinMockPanic => Program::mock_with_id(
            remoting.system(),
            BRIDGE_BUILTIN_ID,
            GearBridgeBuiltinMockPanic,
        ),
    };
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    // Gateway
    let gateway_code_id = remoting.system().submit_code(vft_gateway::WASM_BINARY);
    let init_config = InitConfig {
        receiver_contract_address: [1; 20].into(),
        gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
        eth_client: ETH_CLIENT_ID.into(),
        config: Config {
            gas_to_burn_tokens: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_mint_tokens: 15_000_000_000,
            gas_to_process_mint_request: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
            gas_for_transfer_to_eth_msg: 20_000_000_000,
        },
    };
    let gateway_program_id = VftGatewayFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(gateway_code_id, b"salt")
        .await
        .unwrap();

    // VFT

    let vft_program_id = match vft_mock {
        VftMock::Binary => {
            let vft_code_id = remoting
                .system()
                .submit_code(extended_vft_wasm::WASM_BINARY);
            let vft_program_id = VftFactoryC::new(remoting.clone())
                .new("Token".into(), "Token".into(), 18)
                .send_recv(vft_code_id, b"salt")
                .await
                .unwrap();
            vft_program_id
        }
        mock => {
            let vft = match mock {
                VftMock::FTMockError => {
                    Program::mock_with_id(remoting.system(), TOKEN_ID, FTMockError)
                }
                VftMock::FTMockReturnsFalse => {
                    Program::mock_with_id(remoting.system(), TOKEN_ID, FTMockReturnsFalse)
                }
                _ => unreachable!(),
            };

            assert!(!vft.send_bytes(ADMIN_ID, b"INI").main_failed());
            vft.id()
        }
    };

    Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    }
}

async fn balance_of(
    remoting: &GTestRemoting,
    vft_program_id: ActorId,
    program_id: ActorId,
) -> U256 {
    VftC::new(remoting.clone())
        .balance_of(program_id)
        .recv(vft_program_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_successful_transfer_vara_to_eth() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut vft = VftC::new(remoting.clone());
    let ok = vft
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    vft.grant_burner_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .transfer_vara_to_eth(account_id, vft_program_id, amount, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap()
        .unwrap();
    let expected = (U256::from(1), H160::from([2; 20]));
    assert_eq!(reply, expected);

    let msg_tracker = gateway
        .msg_tracker_state()
        .recv(gateway_program_id)
        .await
        .unwrap();
    assert!(msg_tracker.is_empty());
}

// error in token
#[tokio::test]
async fn test_transfer_fails_due_to_token_panic() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test_with_mocks(Default::default(), VftMock::FTMockError).await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .transfer_vara_to_eth(account_id, vft_program_id, amount, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::ReplyFailure));

    let msg_tracker = gateway
        .msg_tracker_state()
        .recv(gateway_program_id)
        .await
        .unwrap();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
async fn test_transfer_fails_due_to_token_rejecting_request() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test_with_mocks(Default::default(), VftMock::FTMockReturnsFalse).await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .transfer_vara_to_eth(account_id, vft_program_id, amount, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::BurnTokensFailed));

    let msg_tracker = gateway
        .msg_tracker_state()
        .recv(gateway_program_id)
        .await
        .unwrap();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
async fn test_bridge_builtin_panic_with_token_mint() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test_with_mocks(
        BridgeBuiltinMock::GearBridgeBuiltinMockPanic,
        Default::default(),
    )
    .await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut vft = VftC::new(remoting.clone());
    let ok = vft
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    vft.grant_burner_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .transfer_vara_to_eth(account_id, vft_program_id, amount, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::ReplyFailure));

    let msg_tracker = gateway
        .msg_tracker_state()
        .recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(msg_tracker[0].1.status, MessageStatus::MintTokensStep);

    vft.grant_minter_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .handle_interrupted_transfer(msg_tracker[0].0)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::TokensRefunded));
}

#[tokio::test]
async fn test_multiple_transfers() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id1: ActorId = 10001.into();
    let account_id2: ActorId = 10002.into();
    let amount1 = U256::from(10_000_000_000_u64);
    let amount2 = U256::from(5_000_000_000_u64);
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut vft = VftC::new(remoting.clone());
    let ok = vft
        .mint(account_id1, amount1)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = vft
        .mint(account_id2, amount2)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    vft.grant_burner_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply1 = VftGatewayC::new(remoting.clone().with_actor_id(account_id1))
        .transfer_vara_to_eth(account_id1, vft_program_id, amount1, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    let reply2 = VftGatewayC::new(remoting.clone().with_actor_id(account_id2))
        .transfer_vara_to_eth(account_id2, vft_program_id, amount2, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    assert_eq!(reply1, Ok((U256::from(1), H160::from([2; 20]))));
    assert_eq!(reply2, Ok((U256::from(1), H160::from([2; 20]))));
}

#[tokio::test]
async fn test_transfer_vara_to_eth_insufficient_balance() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let excessive_amount = U256::from(20_000_000_000_u64); // More than the available balance
    let gas = 100_000_000_000;
    let eth_token_id = [2; 20].into();

    let mut vft = VftC::new(remoting.clone());
    let ok = vft
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    vft.grant_burner_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let mut gateway = VftGatewayC::new(remoting.clone());
    gateway
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(account_id))
        .transfer_vara_to_eth(account_id, vft_program_id, excessive_amount, eth_token_id)
        .with_gas_limit(gas)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::ReplyFailure));

    // Balance should remain unchanged
    assert_eq!(
        balance_of(&remoting, vft_program_id, account_id).await,
        amount
    )
}

#[tokio::test]
async fn test_mint_tokens_from_eth_client() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let eth_token_id = H160::default();
    let receiver: ActorId = 10_000.into();
    let amount = U256::from(10_000_000_000_u64);

    VftGatewayC::new(remoting.clone())
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    VftC::new(remoting.clone())
        .grant_minter_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    VftGatewayC::new(remoting.clone().with_actor_id(ETH_CLIENT_ID.into()))
        .mint_tokens(eth_token_id, receiver, amount)
        .send_recv(gateway_program_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        balance_of(&remoting, vft_program_id, receiver).await,
        amount
    );
}

#[tokio::test]
async fn test_mint_tokens_from_arbitrary_address() {
    let Fixture {
        remoting,
        gateway_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let eth_token_id = H160::default();
    let receiver: ActorId = 10_000.into();
    let amount = U256::from(10_000_000_000_u64);
    let wrong_address: ActorId = 1_010.into();

    VftGatewayC::new(remoting.clone())
        .map_vara_to_eth_address(vft_program_id, eth_token_id)
        .send_recv(gateway_program_id)
        .await
        .unwrap();

    VftC::new(remoting.clone())
        .grant_minter_role(gateway_program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();

    let reply = VftGatewayC::new(remoting.clone().with_actor_id(wrong_address))
        .mint_tokens(eth_token_id, receiver, amount)
        .send_recv(gateway_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::NotEthClient));
}

#[test]
fn calculate_bridge_builtint() {
    let bytes = hash((b"built/in", 3).encode().as_slice());
    let program_id: ActorId = bytes.into();
    println!("{:?}", program_id);
}

pub fn hash(data: &[u8]) -> [u8; 32] {
    type Blake2b256 = Blake2b<U32>;

    let mut ctx = Blake2b256::new();
    ctx.update(data);
    ctx.finalize().into()
}
