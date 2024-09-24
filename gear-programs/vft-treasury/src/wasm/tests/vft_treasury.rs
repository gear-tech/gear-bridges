use gtest::{Program, System};
use sails_rs::prelude::*;
use utils::{VftTreasury, *};
use vft_treasury_app::services::error::Error;

mod utils;

fn setup_for_test(system: &System) -> (Program<'_>, Program<'_>, Program<'_>) {
    system.init_logger();

    let vft = Program::token(system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_treasury = Program::vft_treasury(system);

    (gear_bridge_builtin, vft, vft_treasury)
}

#[test]
fn test_treasury() {
    let system = System::new();
    let (_gear_bridge_builtin, vft, vft_treasury) = setup_for_test(&system);

    vft_treasury.map_vara_to_eth_address(ADMIN_ID, [2; 20].into(), vft.id());

    let account_id: u64 = 100000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);

    vft.approve(account_id, vft_treasury.id(), amount);

    let reply = vft_treasury.deposit_tokens(
        ADMIN_ID,
        vft.id(),
        account_id.into(),
        amount,
        [3; 20].into(),
        gas,
    );

    let expected = Ok((U256::from(1), H160::from([2; 20])));

    assert_eq!(reply, expected);
    assert!(vft.balance_of(account_id.into()).is_zero());
    assert_eq!(vft.balance_of(vft_treasury.id()), amount);

    vft_treasury
        .withdraw_tokens(
            ETH_CLIENT_ID,
            [2; 20].into(),
            account_id.into(),
            amount,
            gas,
            false,
        )
        .unwrap();

    assert_eq!(vft.balance_of(account_id.into()), amount);
    assert!(vft.balance_of(vft_treasury.id()).is_zero());
}

#[test]
fn test_mapping_does_not_exists() {
    let system = System::new();
    let (_gear_bridge_builtin, vft, vft_treasury) = setup_for_test(&system);

    let account_id: u64 = 100000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);
    vft.approve(account_id, vft_treasury.id(), amount);

    let reply = vft_treasury.deposit_tokens(
        ADMIN_ID,
        vft.id(),
        account_id.into(),
        amount,
        [3; 20].into(),
        gas,
    );

    assert!(reply.is_err());
    assert_eq!(reply.unwrap_err(), Error::NoCorrespondingEthAddress);
}

#[test]
fn test_withdraw_fails_with_bad_origin() {
    let system = System::new();
    let (_gear_bridge_builtin, vft, vft_treasury) = setup_for_test(&system);

    vft_treasury.map_vara_to_eth_address(ADMIN_ID, [2; 20].into(), vft.id());

    let account_id: u64 = 100000;

    let result = vft_treasury.withdraw_tokens(
        ADMIN_ID,
        [2; 20].into(),
        account_id.into(),
        U256::from(42),
        100_000_000_000,
        false,
    );

    assert!(matches!(result, Err(Error::NotEthClient)));
}

#[test]
fn test_anyone_can_deposit() {
    let system = System::new();

    let (_gear_bridge_builtin, vft, vft_treasury) = setup_for_test(&system);

    vft_treasury.map_vara_to_eth_address(ADMIN_ID, [2; 20].into(), vft.id());

    let account0_id: u64 = 100000;
    let account1_id: u64 = 100001;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account0_id.into(), amount);
    vft.mint(ADMIN_ID, account1_id.into(), amount);

    vft.approve(account0_id, vft_treasury.id(), amount);
    vft.approve(account1_id, vft_treasury.id(), amount);

    let reply = vft_treasury.deposit_tokens(
        account1_id,
        vft.id(),
        account0_id.into(),
        amount,
        [3; 20].into(),
        gas,
    );

    let expected = Ok((U256::from(1), H160::from([2; 20])));

    assert_eq!(reply, expected);
    assert!(vft.balance_of(account0_id.into()).is_zero());
    assert_eq!(vft.balance_of(vft_treasury.id()), amount);

    let reply = vft_treasury.deposit_tokens(
        account0_id,
        vft.id(),
        account1_id.into(),
        amount,
        [3; 20].into(),
        gas,
    );
    assert_eq!(reply, expected);
    assert!(vft.balance_of(account1_id.into()).is_zero());
    assert_eq!(vft.balance_of(vft_treasury.id()), amount * 2);
}
