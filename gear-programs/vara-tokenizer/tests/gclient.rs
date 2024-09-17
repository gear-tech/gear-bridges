use gclient::GearApi;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vara_tokenizer_client::traits::*;

async fn init_remoting() -> (GearApi, GClientRemoting, CodeId) {
    let gear_path = option_env!("GEAR_PATH");
    if gear_path.is_none() {
        crate::panic!("the 'GEAR_PATH' environment variable was not set during compile time");
    }
    let api = GearApi::dev_from_path(gear_path.unwrap()).await.unwrap();
    let (code_id, ..) = api.upload_code(vara_tokenizer::WASM_BINARY).await.unwrap();

    let remoting = GClientRemoting::new(api.clone());
    (api, remoting, code_id)
}

#[tokio::test]
#[ignore = "requires run gear node on GEAR_PATH"]
async fn factory_works() {
    // arrange
    let (_api, remoting, program_code_id) = init_remoting().await;

    // act
    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());
    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let vft_client = vara_tokenizer_client::Vft::new(remoting.clone());
    let total_supply = vft_client
        .total_supply()
        .recv(program_id)
        .await
        .expect("Failed");

    // assert
    assert!(total_supply.is_zero());
}

#[tokio::test]
#[ignore = "requires run gear node on GEAR_PATH"]
async fn mint_from_value_works() {
    // arrange
    let (api, remoting, program_code_id) = init_remoting().await;
    let admin_id =
        ActorId::try_from(api.account_id().encode().as_ref()).expect("failed to create actor id");

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = api
        .free_balance(admin_id)
        .await
        .expect("Failed to get free balance");
    let program_initial_balance = api
        .free_balance(program_id)
        .await
        .expect("Failed to get free balance");
    dbg!(initial_balance, program_initial_balance);

    let mint_value = 10_000_000_000_000;

    let mut client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    // act
    client
        .mint_from_value()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to mint from value");

    // assert
    let balance = api
        .free_balance(admin_id)
        .await
        .expect("Failed to get free balance");
    let program_balance = api
        .free_balance(program_id)
        .await
        .expect("Failed to get free balance");
    dbg!(balance, program_balance);

    assert_eq!(program_balance, mint_value + program_initial_balance);
}

#[tokio::test]
#[ignore = "requires run gear node on GEAR_PATH"]
async fn burn_and_return_value_works() {
    let (api, remoting, program_code_id) = init_remoting().await;
    let admin_id =
        ActorId::try_from(api.account_id().encode().as_ref()).expect("failed to create actor id");

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new("Name".into(), "Symbol".into(), 10u8, true)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let program_initial_balance = api
        .free_balance(program_id)
        .await
        .expect("Failed to get free balance");

    let mint_value = 10_000_000_000_000;

    let mut client = vara_tokenizer_client::Tokenizer::new(remoting.clone());
    let vft_client = vara_tokenizer_client::Vft::new(remoting.clone());

    client
        .mint_from_value()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to mint from value");

    let client_balance = vft_client
        .balance_of(admin_id)
        .recv(program_id)
        .await
        .unwrap();

    let balance = api
        .free_balance(admin_id)
        .await
        .expect("Failed to get free balance");
    let program_balance = api
        .free_balance(program_id)
        .await
        .expect("Failed to get free balance");
    dbg!(balance, program_balance, client_balance);
    assert_eq!(program_balance, mint_value + program_initial_balance);
    assert_eq!(client_balance, mint_value.into());

    client
        .burn_and_return_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect("Failed to burn and return value");

    let client_balance = vft_client
        .balance_of(admin_id)
        .recv(program_id)
        .await
        .unwrap();

    let balance = api
        .free_balance(admin_id)
        .await
        .expect("Failed to get free balance");
    let program_balance = api
        .free_balance(program_id)
        .await
        .expect("Failed to get free balance");

    // assert
    dbg!(balance, program_balance, client_balance);
    assert_eq!(program_balance, program_initial_balance);
    assert!(client_balance.is_zero());
}
