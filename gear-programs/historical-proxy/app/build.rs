use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let idl_file_path = {
        let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        path.pop();
        path.pop();
        path.pop();

        path.push("api/gear/");
        path.push("eth_events_electra.idl");

        path
    };

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_external_type("BlockHeader", "ethereum_common::beacon::BlockHeader")
        .with_external_type(
            "BlockBody",
            "ethereum_common::beacon::light::electra::BlockBody",
        )
        .with_external_type(
            "ExecutionPayload",
            "ethereum_common::beacon::light::ExecutionPayload",
        )
        .with_external_type("Error", "eth_events_common::Error")
        .with_external_type("CheckedProofs", "eth_events_common::CheckedProofs")
        .generate_to(
            PathBuf::from(env::var("OUT_DIR").unwrap()).join("eth_events_electra_client.rs"),
        )
        .unwrap();
}
