use gtest::{Program, System};
use sails_rs::{collections::HashMap, prelude::*};

mod utils;
use utils::{
    FTMockError, FTMockReturnsFalse, FTMockReturnsTrue, FTMockWrongReply, GearBridgeBuiltinMock,
    Token, VftGateway, ADMIN_ID, BRIDGE_BUILTIN_ID, FEE, TOKEN_ID,
};

#[test]
fn transfer_vara_to_eth_success() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let bridge_built_in = Program::mock_with_id(&system, BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    // initialise bridge builtin
    assert!(!bridge_built_in.send_bytes(ADMIN_ID, vec![]).main_failed());

    let vft_gateway = Program::vft_gateway(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000 as u64);

    vft.mint(ADMIN_ID, account_id.into(), amount, false);
    vft.grant_burner_role(ADMIN_ID, vft_gateway.id(), false);
    vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into(), false);

    vft_gateway.transfer_vara_to_eth(account_id, vft.id(), amount, [3; 20].into(), false);
    println!("{:?}", vft.balance_of(account_id.into()));
}
