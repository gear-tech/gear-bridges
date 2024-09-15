use gtest::{Program, System};
use sails_rs::prelude::*;

mod utils;
use utils::{BridgingPayment, Token, VftTreasury, ADMIN_ID, FEE, TOKEN_ID};

#[test]
fn deposit_to_treasury_success() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);

    let vft_treasury = Program::vft_treasury(&system);
    let bridging_payment = Program::bridge_payment(&system);

    let account_id: u64 = 10000;
    let amount = U256::from(10_000_000_000_u64);

    vft.mint(ADMIN_ID, account_id.into(), amount, false);
    vft.approve(account_id, vft_treasury.id(), amount, false);
    //vft_gateway.map_vara_to_eth_address(ADMIN_ID, vft.id(), [2; 20].into(), false);

    system.mint_to(account_id, FEE);
    bridging_payment.request_transaction(account_id, amount, [1; 20].into(), vft.id(), false);
    assert_eq!(vft.balance_of(account_id.into()), U256::zero());
}
