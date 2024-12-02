use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {

    let idl_file_path = {
        let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        path.pop();
        path.pop();
        path.pop();
        path.push("api/gear/vft_manager.idl");
        path  
    };

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft-manager.rs"))
        .unwrap();
}
