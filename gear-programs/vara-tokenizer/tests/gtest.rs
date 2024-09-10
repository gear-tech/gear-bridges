mod utils;

use gtest::Program;
use sails_rs::calls::*;
use utils::{init_remoting, ExtendedVftMock, ADMIN_ID, VFT_PROGRAM_ID};
use vara_tokenizer_client::traits::*;

#[tokio::test]
async fn factory_works() {
    let (remoting, program_code_id) = init_remoting();
    let vft_program = Program::mock_with_id(remoting.system(), VFT_PROGRAM_ID, ExtendedVftMock);

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new(vft_program.id()) // Call program's constructor (see app/src/lib.rs:29)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    let vft_adresss = client.vft_address().recv(program_id).await.expect("Failed");
    assert_eq!(vft_adresss, vft_program.id());
}

#[tokio::test]
async fn mint_from_value_works() {
    let (remoting, program_code_id) = init_remoting();
    let vft_program = Program::mock_with_id(remoting.system(), VFT_PROGRAM_ID, ExtendedVftMock);

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new(vft_program.id()) // Call program's constructor (see app/src/lib.rs:29)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 1_000_000_000_000;

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
    assert_eq!(balance, initial_balance - mint_value);
    assert_eq!(program_balance, mint_value);
}

#[tokio::test]
async fn mint_from_value_fails() {
    let (remoting, program_code_id) = init_remoting();
    let vft_program = Program::mock_with_id(remoting.system(), VFT_PROGRAM_ID, ExtendedVftMock);

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new(vft_program.id())
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();

    let initial_balance = remoting.system().balance_of(ADMIN_ID);
    let mint_value = 10_000_000_000_000;

    let mut client = vara_tokenizer_client::Tokenizer::new(remoting.clone());

    let err = client
        .mint_from_value()
        .with_value(mint_value)
        .send_recv(program_id)
        .await
        .expect("Failed send_recv")
        .expect_err("Should fail to mint from value");
    assert_eq!(err, "deposit failed");

    let balance = remoting.system().balance_of(ADMIN_ID);
    let program_balance = remoting.system().balance_of(program_id);
    // TODO asserts not working
    assert_eq!(balance, initial_balance);
    assert_eq!(program_balance, 0);
}

#[tokio::test]
async fn admin_service_works() {
    let (remoting, program_code_id) = init_remoting();
    let vft_program = Program::mock_with_id(remoting.system(), VFT_PROGRAM_ID, ExtendedVftMock);

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let program_id = program_factory
        .new(vft_program.id())
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
