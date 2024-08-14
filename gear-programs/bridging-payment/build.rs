use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let idl_file_path = PathBuf::from("../vft-gateway/src/wasm/vft-gateway.idl");

    let client_rs_file_path = out_dir_path.join("vft-gateway.rs");

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(client_rs_file_path)
        .unwrap();
    let idl_file_path = out_dir_path.join("vft.idl");

    let client_rs_file_path = out_dir_path.join("vft.rs");

    git_download::repo("https://github.com/gear-foundation/standards")
        .branch_name("master")
        .add_file("extended-vft/wasm/extended_vft.idl", &idl_file_path)
        .exec()
        .unwrap();

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(client_rs_file_path)
        .unwrap();
}
