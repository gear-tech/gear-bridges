use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

fn main() {
    // Use release-equivalent WASM settings to fit the runtime code limit.
    // These overrides affect only this build script's child Cargo, not other programs.
    env::set_var("CARGO_PROFILE_DEV_LTO", "fat");
    env::set_var("CARGO_PROFILE_DEV_CODEGEN_UNITS", "1");
    env::set_var("CARGO_PROFILE_DEV_DEBUG_ASSERTIONS", "false");
    env::set_var("CARGO_PROFILE_DEV_OVERFLOW_CHECKS", "false");
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
    sails_idl_gen::generate_idl_to_file::<vft_manager_app::Program>(idl_path).unwrap();
}
