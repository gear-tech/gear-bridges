use sails_rs::{calls::*, gtest::calls::*, ActorId};

use vara_tokenizer_client::traits::*;

const ACTOR_ID: u64 = 42;

#[tokio::test]
async fn factory_works() {
    let remoting = GTestRemoting::new(ACTOR_ID.into());
    remoting.system().init_logger();

    // Submit program code into the system
    let program_code_id = remoting.system().submit_code(vara_tokenizer::WASM_BINARY);

    let program_factory = vara_tokenizer_client::VaraTokenizerFactory::new(remoting.clone());

    let _program_id = program_factory
        .new(ActorId::zero()) // Call program's constructor (see app/src/lib.rs:29)
        .send_recv(program_code_id, b"salt")
        .await
        .unwrap();
}
