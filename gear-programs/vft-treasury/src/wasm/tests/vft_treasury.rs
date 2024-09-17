use gtest::{Program, System};
use sails_rs::prelude::*;
use utils::{VftTreasury, *};

mod utils;

#[test]
fn test_lock_unlock_assets() {
    let system = System::new();
    system.init_logger();
    let eth_token_id = H160::random();
    let vft = Program::token(&system, TOKEN_ID);
    let vft_treasury = Program::vft_treasury(&system);

    vft_treasury.add_ethereum_to_vara_mapping(ADMIN_ID, eth_token_id, vft.id());

    let account_id = 10000;
    let amount = 10_000_000_000_u64;

    vft.mint(ADMIN_ID, account_id.into(), U256::from(amount));
    vft.approve(account_id, vft_treasury.id(), U256::from(amount / 2));

    let tok = vft_treasury.deposit(
        BRIDGE_SERVICE_ID.into(),
        account_id.into(),
        vft.id(),
        (amount / 2).into(),
        H160::random(),
    );

    assert_eq!(tok, eth_token_id);
    assert_eq!(vft.balance_of(account_id.into()), U256::from(amount / 2));

    vft_treasury.withdraw(
        ETH_CLIENT_ID,
        eth_token_id,
        account_id.into(),
        U256::from(amount / 2),
    );

    assert_eq!(vft.balance_of(account_id.into()), U256::from(amount));
}

/*
#[test]

fn test_lock_unlock_assets() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let vft_treasury = Program::vft_treasury(&system);

    let account_id = 10000;
    let amount = 10_000_000_000_u64;
    vft.mint(ADMIN_ID, account_id.into(), U256::from(amount));
    vft.approve(account_id, vft_treasury.id(), U256::from(amount / 2));

    vft_treasury.deposit(account_id, vft.id(), U256::from(amount / 2));

    assert_eq!(vft.balance_of(vft_treasury.id()), U256::from(amount / 2));
    assert_eq!(vft.balance_of(account_id.into()), U256::from(amount / 2));

    vft_treasury.withdraw(
        ETH_CLIENT_ID,
        vft.id(),
        account_id.into(),
        U256::from(amount / 2),
    );

    assert!(vft.balance_of(vft_treasury.id()).is_zero());
    assert_eq!(vft.balance_of(account_id.into()), U256::from(amount));
}

#[test]

fn test_lock_unlock_assets_two_accounts() {
    let system = System::new();
    system.init_logger();

    let vft = Program::token(&system, TOKEN_ID);
    let vft_treasury = Program::vft_treasury(&system);

    let account0_id = 10000;
    let account1_id = 10001;
    let amount = 10_000_000_000_u64;
    vft.mint(ADMIN_ID, account0_id.into(), U256::from(amount));
    vft.approve(account0_id, vft_treasury.id(), U256::from(amount));

    vft_treasury.deposit(account0_id, vft.id(), U256::from(amount));

    assert_eq!(vft.balance_of(vft_treasury.id()), U256::from(amount));
    assert!(vft.balance_of(account0_id.into()).is_zero());

    vft_treasury.withdraw(
        ETH_CLIENT_ID,
        vft.id(),
        account1_id.into(),
        U256::from(amount / 2),
    );

    vft_treasury.withdraw(
        ETH_CLIENT_ID,
        vft.id(),
        account0_id.into(),
        U256::from(amount / 2),
    );

    assert!(vft.balance_of(vft_treasury.id()).is_zero());
    assert_eq!(vft.balance_of(account1_id.into()), U256::from(amount / 2));
    assert_eq!(vft.balance_of(account0_id.into()), U256::from(amount / 2));
}
*/
