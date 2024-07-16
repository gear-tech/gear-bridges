use std::{env, path::PathBuf};

fn main() {
    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let idl_file_path = out_dir_path.join("vft_master.idl");

    let client_rs_file_path = out_dir_path.join("vft_master.rs");

    git_download::repo("https://github.com/gear-foundation/standards")
        .branch_name("lm-vft-master")
        .add_file("vft-master/wasm/vft_master.idl", &idl_file_path)
        .exec()
        .unwrap();

    sails_client_gen::generate_client_from_idl(&idl_file_path, client_rs_file_path).unwrap();
}
