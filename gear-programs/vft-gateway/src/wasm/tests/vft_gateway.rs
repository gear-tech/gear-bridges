use gtest::{Program, System};

use sails_rs::prelude::*;
use vft_gateway_app::services::{error::Error, msg_tracker::MessageStatus};

mod utils;
use blake2::{digest::typenum::U32, Blake2b, Digest};
use utils::{
    FTMockError, FTMockReturnsFalse, GearBridgeBuiltinMock, GearBridgeBuiltinMockPanic, Token,
    VftGateway, ADMIN_ID, BRIDGE_BUILTIN_ID, ETH_CLIENT_ID, TOKEN_ID,
};
type Blake2b256 = Blake2b<U32>;
use gear_core::ids::ProgramId;

#[test]
#[ignore = "Fails for now"]
fn test_successful_transfer_vara_to_eth() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());

    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    let exp_reply: Result<(U256, H160), Error> = Ok((U256::from(1), H160::from([2; 20])));
    assert_eq!(reply, exp_reply);
    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
}

// error in token
#[tokio::test]
#[ignore = "Fails for now"]
async fn test_transfer_fails_due_to_token_panic() {
    let system = System::new();
    system.init_logger();

    let vft = Program::mock_with_id(&system, TOKEN_ID, FTMockError);
    assert!(!vft.send_bytes(ADMIN_ID, b"INI").main_failed());
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::ReplyFailure));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
#[ignore = "Fails for now"]
async fn test_transfer_fails_due_to_token_rejecting_request() {
    let system: System = System::new();
    system.init_logger();

    let vft = Program::mock_with_id(&system, TOKEN_ID, FTMockReturnsFalse);
    assert!(!vft.send_bytes(ADMIN_ID, b"INI").main_failed());
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::BurnTokensFailed));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
#[ignore = "Fails for now"]
async fn test_bridge_builtin_panic_with_token_mint() {
    let system: System = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMockPanic);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::ReplyFailure));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert_eq!(msg_tracker[0].1.status, MessageStatus::MintTokensStep);

    // grant minter role and continue transaction
    vft.grant_minter_role(ADMIN_ID, vft_gateway.id());

    let reply = vft_gateway.handle_interrupted_transfer(account_id, msg_tracker[0].0);
    assert_eq!(reply, Err(Error::TokensRefunded));
}

#[tokio::test]
#[ignore = "Fails for now"]
async fn test_multiple_transfers() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_build_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!bridge_build_in.send_bytes(ADMIN_ID, b"INIT").main_failed());
    let vft_gateway = Program::vft_gateway(&system);

    let account_id1: u64 = 10001;
    let account_id2: u64 = 10002;
    let amount1 = U256::from(10_000_000_000_u64);
    let amount2 = U256::from(5_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id1.into(), amount1);
    vft.mint(ADMIN_ID, account_id2.into(), amount2);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());

    // Execute multiple transfers simultaneously
    let reply1 = vft_gateway.transfer_vara_to_eth(
        account_id1,
        vft.id(),
        amount1,
        [3; 20].into(),
        gas,
        false,
    );
    let reply2 = vft_gateway.transfer_vara_to_eth(
        account_id2,
        vft.id(),
        amount2,
        [4; 20].into(),
        gas,
        false,
    );

    assert_eq!(reply1, Ok((U256::from(1), H160::from([2; 20]))));
    assert_eq!(reply2, Ok((U256::from(1), H160::from([2; 20]))));
}

#[test]
#[ignore = "Fails for now"]
fn test_transfer_vara_to_eth_insufficient_balance() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);
    let excessive_amount = U256::from(20_000_000_000_u64); // More than the available balance
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());

    let reply = vft_gateway.transfer_vara_to_eth(
        account_id,
        vft.id(),
        excessive_amount,
        [3; 20].into(),
        gas,
        false,
    );
    assert_eq!(reply, Err(Error::ReplyFailure));

    let balance = vft.balance_of(account_id.into());
    assert_eq!(balance, amount); // Balance should remain unchanged
}

#[test]
fn test_mint_tokens_from_eth_client() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);
    let eth_token_id = H160::default();
    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), eth_token_id);

    let receiver: u64 = 10_000;
    let amount = U256::from(10_000_000_000_u64);

    vft.grant_minter_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.mint_tokens(ETH_CLIENT_ID, eth_token_id, amount, receiver.into(), false);

    let balance = vft.balance_of(receiver.into());
    assert_eq!(balance, amount);
}

#[test]
fn test_mint_tokens_from_arbitrary_address() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_gateway = Program::vft_gateway(&system);
    let eth_token_id = H160::default();
    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), eth_token_id);

    let receiver: u64 = 10_000;
    let amount = U256::from(10_000_000_000_u64);

    vft.grant_minter_role(ADMIN_ID, vft_gateway.id());

    let wrong_address = 1_010;

    vft_gateway.mint_tokens(wrong_address, eth_token_id, amount, receiver.into(), true);
}

#[test]
fn test_eth_client() {
    let system = System::new();
    system.init_logger();

    let vft_gateway = Program::vft_gateway(&system);

    assert_eq!(vft_gateway.eth_client(ADMIN_ID), ETH_CLIENT_ID.into());

    // anyone is able to get the eth client address
    let wrong_address = 1_010;
    assert_eq!(vft_gateway.eth_client(wrong_address), ETH_CLIENT_ID.into());

    // non-admin user isn't allowed to change eth client
    assert!(!vft_gateway.update_eth_client(wrong_address, ADMIN_ID.into()));

    assert!(vft_gateway.update_eth_client(ADMIN_ID, ADMIN_ID.into()));

    assert_eq!(vft_gateway.eth_client(ETH_CLIENT_ID), ADMIN_ID.into());
    assert_eq!(vft_gateway.eth_client(wrong_address), ADMIN_ID.into());
}

#[test]
fn calculate_bridge_builtint() {
    let bytes = hash((b"built/in", 3).encode().as_slice());
    let program_id: ProgramId = bytes.into();
    println!("{:?}", program_id);
}

pub fn hash(data: &[u8]) -> [u8; 32] {
    let mut ctx = Blake2b256::new();
    ctx.update(data);
    ctx.finalize().into()
}
