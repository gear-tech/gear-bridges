use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vara_tokenizer_client::traits::*;

pub const ADMIN_ID: u64 = 42;

pub fn init_remoting() -> (GTestRemoting, CodeId) {
    let remoting = GTestRemoting::new(ADMIN_ID.into());
    remoting.system().mint_to(ADMIN_ID, 100_000_000_000_000);
    remoting.system().init_logger();

    // Submit program code into the system
    let program_code_id = remoting.system().submit_code(vara_tokenizer::WASM_BINARY);
    (remoting, program_code_id)
}

#[tokio::test]
async fn factory_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    let total_supply = client
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

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 10_000_000_000_000;

    let mut client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    client
        .mint_from_value()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to mint from value");

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);
    // TODO update test after next `gtest` release, fixing gas issues
    // see https://github.com/gear-tech/gear/pull/4200 and other `gtest` related PRs
    assert_eq!(balance, initial_balance - mint_value);
    assert_eq!(program_balance, mint_value);
}

#[tokio::test]
async fn burn_and_return_value_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 10_000_000_000_000;

    let mut client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    client
        .mint_from_value()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to mint from value");

    let client_balance = client
        .balance_of(ADMIN_ID.into())
        .recv(program_id)
        .await
        .unwrap();

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);
    assert_eq!(balance, initial_balance - mint_value);
    assert_eq!(program_balance, mint_value);
    assert_eq!(client_balance, mint_value.into());

    client
        .burn_and_return_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to burn and return value");

    let client_balance = client
        .balance_of(ADMIN_ID.into())
        .recv(program_id)
        .await
        .unwrap();

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);
    // TODO update test after next `gtest` release, fixing gas issues
    // see https://github.com/gear-tech/gear/pull/4200 and other `gtest` related PRs
    dbg!(balance, program_balance, client_balance);
    assert!(client_balance.is_zero());
    // assert_eq!(balance, initial_balance);
    // assert_eq!(program_balance, 0);
}

#[tokio::test]
async fn admin_service_works() {
    let (remoting, program_code_id) = init_remoting();

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let mut client = vara_tokenizer_client::Admin::new(remoting.clone());

    let admins = client.admins().recv(program_id).await.expect("Failed");
    assert_eq!(admins.as_slice(), &[ADMIN_ID.into()]);

    // grant admin role
    let new_admin_id = 2000;
    client
        .grant_admin_role(new_admin_id.into())
        .send_recv(program_id)
        .await
        .expect("Failed to grant admin role");

    let admins = client.admins().recv(program_id).await.expect("Failed");
    assert_eq!(admins.as_slice(), &[ADMIN_ID.into(), new_admin_id.into()]);

    // revoke admin role from ADMIN_ID
    client
        .revoke_admin_role(ADMIN_ID.into())
        .send_recv(program_id)
        .await
        .expect("Failed to revoke admin role");

    let admins = client.admins().recv(program_id).await.expect("Failed");
    assert_eq!(admins.as_slice(), &[new_admin_id.into()]);

    // ADMIN_ID is not admin
    let _err = client
        .revoke_admin_role(new_admin_id.into())
        .send_recv(program_id)
        .await
        .expect_err("Should fail to revoke admin role");
}
