use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let idl_file_path = out_dir_path.join("checkpoint_light_client.idl");

    // Generate IDL file for the program
    sails_idl_gen::generate_idl_to_file::<checkpoint_light_client_app::CheckpointLightClientProgram>(&idl_file_path).unwrap();

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_external_type("BlockHeader", "ethereum_common::beacon::BlockHeader")
        .with_external_type("Block", "ethereum_common::beacon::light::Block")
        .with_external_type("BlockBody", "ethereum_common::beacon::light::BlockBody")
        .with_external_type(
            "ExecutionPayload",
            "ethereum_common::beacon::light::ExecutionPayload",
        )
        .with_external_type("Init", "checkpoint_light_client_io::Init")
        .with_external_type("Update", "checkpoint_light_client_io::Update")
        .with_external_type("Error", "checkpoint_light_client_io::Error")
        .with_external_type("ReplayBack", "checkpoint_light_client_io::ReplayBack")
        .with_external_type(
            "ReplayBackError",
            "checkpoint_light_client_io::ReplayBackError",
        )
        .with_external_type(
            "ReplayBackStatus",
            "checkpoint_light_client_io::ReplayBackStatus",
        )
        .with_mocks("mocks")
        .generate_to(
            PathBuf::from(env::var("OUT_DIR").unwrap()).join("checkpoint_light_client_client.rs"),
        )
        .unwrap();
}
