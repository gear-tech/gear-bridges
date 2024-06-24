use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let idl_file_path = out_dir_path.join("erc20.idl");

    let client_rs_file_path = out_dir_path.join("erc20.rs");

    git_download::repo("https://github.com/gear-foundation/standards")
        .branch_name("master")
        .add_file("gear-erc20/wasm/erc20.idl", &idl_file_path)
        .exec()
        .unwrap();

    sails_client_gen::generate_client_from_idl(&idl_file_path, client_rs_file_path).unwrap();
}
