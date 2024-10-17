use gtest::System;
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use wrapped_vara_client::traits::*;

pub const ADMIN_ID: u64 = 42;

pub fn init_remoting() -> (GTestRemoting, CodeId) {
    let system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);
    let remoting = GTestRemoting::new(system, ADMIN_ID.into());

    // Submit program code into the system
    let program_code_id = remoting.system().submit_code(wrapped_vara::WASM_BINARY);
    (remoting, program_code_id)
}

#[tokio::test]
async fn factory_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = wrapped_vara_client::WrappedVaraFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let vft_client = wrapped_vara_client::Vft::new(remoting.clone());

    let total_supply = vft_client
        .total_supply()
        .recv(program_id)
        .await
        .expect("Failed");

    // assert
    assert!(total_supply.is_zero());
}

#[tokio::test]
async fn mint_from_value_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = wrapped_vara_client::WrappedVaraFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 10_000_000_000_000;

    let program_initial_balance = remoting.system().balance_of(program_id);

    let mut client = wrapped_vara_client::Tokenizer::new(remoting.clone());

    let minted_value = client
        .mint()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv");

    assert_eq!(mint_value, minted_value);

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);

    assert!(balance < initial_balance - mint_value);
    assert_eq!(program_balance, mint_value + program_initial_balance);
}

#[tokio::test]
async fn burn_and_return_value_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = wrapped_vara_client::WrappedVaraFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 10_000_000_000_000;

    let program_initial_balance = remoting.system().balance_of(program_id);

    let mut client = wrapped_vara_client::Tokenizer::new(remoting.clone());
    let vft_client = wrapped_vara_client::Vft::new(remoting.clone());

    client
        .mint()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv");

    let client_balance = vft_client
        .balance_of(ADMIN_ID.into())
        .recv(program_id)
        .await
        .unwrap();

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);
    assert!(balance < initial_balance - mint_value);
    assert_eq!(program_balance, mint_value + program_initial_balance);
    assert_eq!(client_balance, mint_value.into());

    client
        .burn(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv");

    let client_balance = vft_client
        .balance_of(ADMIN_ID.into())
        .recv(program_id)
        .await
        .unwrap();

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);

    assert!(client_balance.is_zero());
    assert!(balance > initial_balance - mint_value);
    assert_eq!(program_balance, program_initial_balance);
}
