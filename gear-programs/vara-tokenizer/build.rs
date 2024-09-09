use sails_client_gen::ClientGenerator;
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

fn main() {
    let idl_file_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("extended_vft.idl");
    ClientGenerator::from_idl_path(&idl_file_path)
        .with_mocks("mockall")
        .generate_to(PathBuf::from(env::var("OUT_DIR").unwrap()).join("extended_vft_client.rs"))
        .unwrap();

    sails_rs::build_wasm();

    if env::var("__GEAR_WASM_BUILDER_NO_BUILD").is_ok() {
        return;
    }

    let bin_path_file = File::open(".binpath").unwrap();
    let mut bin_path_reader = BufReader::new(bin_path_file);
    let mut bin_path = String::new();
    bin_path_reader.read_line(&mut bin_path).unwrap();

    let mut idl_path = PathBuf::from(bin_path);
    idl_path.set_extension("idl");
    sails_idl_gen::generate_idl_to_file::<vara_tokenizer_app::VaraTokenizerProgram>(idl_path)
        .unwrap();
}
