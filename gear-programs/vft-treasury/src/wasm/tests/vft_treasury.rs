use gtest::{Program, System};
use sails_rs::prelude::*;
use utils::{VftTreasury, *};

mod utils;

#[test]
fn test_treasury() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let gear_bridge_builtin =
        Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    assert!(!gear_bridge_builtin
        .send_bytes(ADMIN_ID, b"INIT")
        .main_failed());

    let vft_treasury = Program::vft_treasury(&system);

    let account_id: u64 = 100000;
    let amount = U256::from(10_000_000_000_u64);
    let gas = 100_000_000_000;

    vft.mint(ADMIN_ID, account_id.into(), amount);

    vft.approve(account_id, vft_treasury.id(), amount);

    vft_treasury.map_vara_to_eth_address(ADMIN_ID, [2; 20].into(), vft.id());

    let reply = vft_treasury.deposit_tokens(
        BRIDGE_SERVICE_ID,
        vft.id(),
        account_id.into(),
        amount,
        [3; 20].into(),
        gas,
        false,
    );

    let expected = Ok((U256::from(1), H160::from([2; 20])));

    assert_eq!(reply, expected);

    vft_treasury.withdraw_tokens(
        ETH_CLIENT_ID,
        [2; 20].into(),
        account_id.into(),
        amount,
        gas,
        false,
        false,
    );

    assert_eq!(vft.balance_of(account_id.into()), amount);
}
