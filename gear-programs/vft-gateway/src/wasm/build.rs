use sails_client_gen::ClientGenerator;
use sails_idl_gen::program;
use std::{env, fs::File, path::PathBuf};
use vft_gateway_app::Program;

fn main() {
    gwasm_builder::build();

    let manifest_dir_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let idl_file_path = manifest_dir_path.join("vft-gateway.idl");

    let idl_file = File::create(idl_file_path.clone()).unwrap();

    program::generate_idl::<Program>(idl_file).unwrap();

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("vft_gateway_client.rs"))
        .unwrap();
}
