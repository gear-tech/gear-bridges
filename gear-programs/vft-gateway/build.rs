use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let client_rs_file_path = out_dir_path.join("vft.rs");

    #[cfg(not(target_family = "windows"))]
    let idl_file_path = out_dir_path.join("vft.idl");
    #[cfg(not(target_family = "windows"))]
    git_download::repo("https://github.com/gear-foundation/standards")
        .branch_name("master")
        .add_file("extended-vft/wasm/extended_vft.idl", &idl_file_path)
        .exec()
        .unwrap();

    // use local copy of `vft.idl` to build on windows
    #[cfg(target_family = "windows")]
    let idl_file_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("vft.idl");

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(client_rs_file_path)
        .unwrap();
}
