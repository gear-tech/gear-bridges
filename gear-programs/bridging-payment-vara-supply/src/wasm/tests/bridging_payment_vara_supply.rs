use gtest::{Log, Program, System};
use sails_rs::prelude::*;

mod utils;
use utils::{
    BridgingPayment, GearBridgeBuiltinMock, Token, VftTreasury, ADMIN_ID, BRIDGE_BUILTIN_ID, FEE,
    TOKEN_ID, VFT_TREASURY_ID,
};

#[test]
fn deposit_to_treasury_success() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_built_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    // Init bridge builtin
    assert!(!bridge_built_in.send_bytes(ADMIN_ID, vec![]).main_failed());

    let vft_treasury = Program::vft_treasury(&system);
    let bridging_payment = Program::bridge_payment(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);

    vft.mint(ADMIN_ID, account_id.into(), amount, false);
    vft.approve(account_id, vft_treasury.id(), amount, false);
    vft_treasury.map_vara_to_eth_address(ADMIN_ID, [2; 20].into(), vft.id(), false);

    system.mint_to(account_id, FEE);
    bridging_payment.request(account_id, amount, [1; 20].into(), vft.id(), false);
    assert_eq!(vft.balance_of(account_id.into()), U256::zero());
    assert_eq!(vft.balance_of(VFT_TREASURY_ID.into()), amount);

    // Claim fee
    bridging_payment.reclaim_fee(ADMIN_ID, false);
    system
        .get_mailbox(ADMIN_ID)
        .claim_value(Log::builder().dest(ADMIN_ID))
        .unwrap();
    assert_eq!(system.balance_of(ADMIN_ID), FEE);
}
