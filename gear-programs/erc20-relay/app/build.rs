use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    path.pop();
    path.pop();

    let idl_file_path = path.join("vft-gateway/src/wasm/vft-gateway.idl");

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft-gateway.rs"))
        .unwrap();
}
