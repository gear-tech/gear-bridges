use gclient::GearApi;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vara_tokenizer_client::traits::*;
use vft_client::traits::*;

const EXTENDED_VFT_WASM_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/extended_vft_wasm.opt.wasm");

async fn init_remoting() -> (GearApi, GClientRemoting, CodeId, CodeId) {
    let gear_path = option_env!("GEAR_PATH");
    if gear_path.is_none() {
        crate::panic!("the 'GEAR_PATH' environment variable was not set during compile time");
    }
    let api = GearApi::dev_from_path(gear_path.unwrap()).await.unwrap();
    let (code_id, ..) = api.upload_code(vara_tokenizer::WASM_BINARY).await.unwrap();

    let (vft_code_id, ..) = api
        .upload_code_by_path(EXTENDED_VFT_WASM_PATH)
        .await
        .unwrap();

    let remoting = GClientRemoting::new(api.clone());
    (api, remoting, code_id, vft_code_id)
}

#[tokio::test]
#[ignore = "requires run gear node on GEAR_PATH"]
async fn factory_works() {
    // arrange
    let (api, remoting, program_code_id, vft_code_id) = init_remoting().await;
    let _admin_id =
        ActorId::try_from(api.account_id().encode().as_ref()).expect("failed to create actor id");

    // act
    let vft_factory = vft_client::ExtendedVftFactory::new(remoting.clone());
    let vft_program_id = vft_factory
        .new("Name".into(), "Symbol".into(), 10u8)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());
    let program_id = program_factory
        .new(vft_program_id)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let client = vara_tokenizer_client::Tokenizer::new(remoting.clone());
    let vft_adresss = client.vft_address().recv(program_id).await.expect("Failed");

    // assert
    assert_eq!(vft_adresss, vft_program_id);
}

#[tokio::test]
#[ignore = "requires run gear node on GEAR_PATH"]
async fn mint_from_value_works() {
    // arrange
    let (api, remoting, program_code_id, vft_code_id) = init_remoting().await;
    let admin_id =
        ActorId::try_from(api.account_id().encode().as_ref()).expect("failed to create actor id");

    let vft_factory = vft_client::ExtendedVftFactory::new(remoting.clone());
    let vft_program_id = vft_factory
        .new("Name".into(), "Symbol".into(), 10u8)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new(vft_program_id)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let mut vft_client = vft_client::Vft::new(remoting.clone());
    vft_client
        .grant_minter_role(program_id)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    vft_client
        .grant_burner_role(program_id)
        .send_recv(vft_program_id)
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

    assert_eq!(program_balance, mint_value + 1_000_000_000_000); // ?
}
