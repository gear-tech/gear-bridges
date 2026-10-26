use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let idl_file_path = out_dir_path.join("vft_vara.idl");

    // Generate IDL file for the program
    sails_idl_gen::generate_idl_to_file::<vft_vara_app::Program>(&idl_file_path).unwrap();
    let original_idl = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../../api/gear/vft_vara_v1.idl");
    println!("cargo:rerun-if-changed={}", original_idl.display());
    assert_eq!(
        std::fs::read(&idl_file_path).unwrap(),
        std::fs::read(&original_idl).unwrap(),
        "Pinned v1 source/dependencies must retain the original IDL bytes"
    );

    // Generate client code from IDL file
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mocks")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft_vara_client.rs"))
        .unwrap();
}
