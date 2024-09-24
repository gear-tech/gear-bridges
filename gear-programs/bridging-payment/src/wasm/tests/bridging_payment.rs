use gtest::{Program, System};
use sails_rs::prelude::*;

mod utils;
use utils::{
    BridgingPayment, GearBridgeBuiltinMock, Token, VftGateway, ADMIN_ID, BRIDGE_BUILTIN_ID, FEE,
    TOKEN_ID,
};

#[test]
fn transfer_tokens_success() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_built_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    // initialise bridge builtin
    assert!(!bridge_built_in.send_bytes(ADMIN_ID, vec![]).main_failed());

    let vft_gateway = Program::vft_gateway(&system);
    let bridging_payment = Program::bridge_payment(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);

    vft.mint(ADMIN_ID, account_id.into(), amount, false);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id(), false);
    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into(), false);

    system.mint_to(account_id, FEE);
    bridging_payment.request_to_gateway(account_id, amount, [1; 20].into(), vft.id(), false);
    assert_eq!(vft.balance_of(account_id.into()), U256::zero());
}
