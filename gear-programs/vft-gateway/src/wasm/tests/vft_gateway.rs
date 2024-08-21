use gtest::{Program, System};

use sails_rs::prelude::*;
use vft_gateway_app::services::{error::Error, msg_tracker::MessageStatus};

mod utils;
use blake2::{digest::typenum::U32, Blake2b, Digest};
use utils::{
    FTMockError, FTMockReturnsFalse, GearBridgeBuiltinMock, GearBridgeBuiltinMockPanic, Token,
    VftGateway, ADMIN_ID, BRIDGE_BUILTIN_ID, TOKEN_ID,
};
type Blake2b256 = Blake2b<U32>;
use gear_core::ids::ProgramId;
#[test]
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
    let amount = U256::from(10_000_000_000 as u64);
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
    let amount = U256::from(10_000_000_000 as u64);
    let gas = 100_000_000_000;

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::ReplyError));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
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
    let amount = U256::from(10_000_000_000 as u64);
    let gas = 100_000_000_000;

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::BurnTokensFailed));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
}

#[tokio::test]
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
    let amount = U256::from(10_000_000_000 as u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());
    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, false);
    assert_eq!(reply, Err(Error::ReplyError));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert_eq!(msg_tracker[0].1.status, MessageStatus::MintTokensStep);

    // grant minter role and continue transaction
    vft.grant_minter_role(ADMIN_ID, vft_gateway.id());

    let reply = vft_gateway.handle_interrupted_transfer(account_id, msg_tracker[0].0);
    assert_eq!(reply, Err(Error::TokensRefundedError));
}

#[tokio::test]
async fn test_multiple_transfers() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_build_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!bridge_build_in.send_bytes(ADMIN_ID, b"INIT").main_failed());
    let vft_gateway = Program::vft_gateway(&system);

    let account_id1: u64 = 10001;
    let account_id2: u64 = 10002;
    let amount1 = U256::from(10_000_000_000 as u64);
    let amount2 = U256::from(5_000_000_000 as u64);
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
    let amount = U256::from(10_000_000_000 as u64);
    let excessive_amount = U256::from(20_000_000_000 as u64); // More than the available balance
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
    assert_eq!(reply, Err(Error::ReplyError));

    let balance = vft.balance_of(account_id.into());
    assert_eq!(balance, amount); // Balance should remain unchanged
}

#[tokio::test]
async fn test_transfer_fails_due_to_gas_depletion_after_bridge_reply() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_build_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!bridge_build_in.send_bytes(ADMIN_ID, b"INIT").main_failed());
    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10001;
    let amount = U256::from(10_000_000_000 as u64);
    let gas = 50_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id());

    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into());

    // gas ended after contract receives reply from bridge builtin

    let reply =
        vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), gas, true);
    assert_eq!(reply, Err(Error::MessageFailed));

    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert_eq!(
        msg_tracker[0].1.status,
        MessageStatus::BridgeResponseReceived(Some(U256::one()))
    );

    let reply = vft_gateway.handle_interrupted_transfer(account_id, msg_tracker[0].0);
    let exp_reply: Result<(U256, H160), Error> = Ok((U256::from(1), H160::from([2; 20])));
    assert_eq!(reply, exp_reply);
    let msg_tracker = vft_gateway.get_msg_tracker_state();
    assert!(msg_tracker.is_empty());
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