use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let mut dir_manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    dir_manifest.pop();
    dir_manifest.pop();
    dir_manifest.pop();

    dir_manifest.push("api");
    dir_manifest.push("gear");
    let path_idl_file = dir_manifest.join("vft_manager.idl");

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&path_idl_file)
        // .with_mocks("mocks")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft_manager_client.rs"))
        .unwrap();
}
