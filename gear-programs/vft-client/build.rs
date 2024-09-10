use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let idl_file_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("extended_vft.idl");

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft_client.rs"))
        .unwrap();
}
