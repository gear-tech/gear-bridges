use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let idl_file_path = out_dir_path.join("ethereum_event_client.idl");

    // Generate IDL file for the program
    sails_idl_gen::generate_idl_to_file::<ethereum_event_client_app::EthereumEventClientProgram>(
        &idl_file_path,
    )
    .unwrap();

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .with_external_type("BlockHeader", "ethereum_common::beacon::BlockHeader")
        .with_external_type(
            "BlockBody",
            "ethereum_common::beacon::light::electra::BlockBody",
        )
        .with_external_type(
            "ExecutionPayload",
            "ethereum_common::beacon::light::ExecutionPayload",
        )
        .generate_to(
            PathBuf::from(env::var("OUT_DIR").unwrap()).join("ethereum_event_client_client.rs"),
        )
        .unwrap();
}
