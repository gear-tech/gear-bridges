use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
   
    let idl_file_path = PathBuf::from("erc20_relay.idl");

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .with_external_type("BlockHeader", "ethereum_common::beacon::BlockHeader")
        .with_external_type("Block", "ethereum_common::beacon::light::Block")
        .with_external_type("BlockBody", "ethereum_common::beacon::light::BlockBody")
        .with_external_type(
            "ExecutionPayload",
            "ethereum_common::beacon::light::ExecutionPayload",
        )
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("erc20_relay.rs"))
        .unwrap();

    let idl_file_path = {
        let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        path.pop();
        path.pop();

        path.push("vft-manager");
        path.push("vft_manager.idl");

        path
    };

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft-manager.rs"))
        .unwrap();
}
