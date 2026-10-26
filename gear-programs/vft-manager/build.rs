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
    if let Some((original, optimized)) = sails_rs::build_wasm() {
        // Start from the compiler output on every build: optimizing the previous optimized
        // artifact again would make clean and incremental builds produce different CodeIds.
        let mut optimizer = gear_wasm_builder::optimize::Optimizer::new(&original).unwrap();
        optimizer.insert_stack_end_export().unwrap();
        optimizer.strip_custom_sections();
        optimizer.strip_exports();
        optimizer.flush_to_file(&optimized);
        // Gear's default -O4 exceeds the runtime's instrumented-code limit for this program.
        gear_wasm_builder::optimize::optimize_wasm(&optimized, &optimized, "z", true).unwrap();
        // Gas instrumentation expands control-flow blocks; raw byte size is not the limit.
        let status = std::process::Command::new("wasm-opt")
            .arg(&optimized)
            .args([
                "--flatten",
                "--rereloop",
                "-Oz",
                "--converge",
                "-mvp",
                "--enable-sign-ext",
                "--enable-mutable-globals",
                "--limit-segments",
                "--pass-arg=limit-segments@1024",
                "--zero-filled-memory",
                "--dae",
                "--vacuum",
            ])
            .arg("-o")
            .arg(&optimized)
            .status()
            .expect("failed to run manager control-flow optimization");
        assert!(status.success(), "manager control-flow optimization failed");
    }

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
