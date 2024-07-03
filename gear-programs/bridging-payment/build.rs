use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let idl_file_path = PathBuf::from("../vft-gateway/src/wasm/vft-gateway.idl");

    let client_rs_file_path = out_dir_path.join("vft-gateway.rs");

    sails_client_gen::generate_client_from_idl(&idl_file_path, client_rs_file_path).unwrap();
}
