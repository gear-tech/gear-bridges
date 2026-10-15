fn main() {
    #[cfg(feature = "legacy-prover")]
    go_bindings();
}

#[cfg(feature = "legacy-prover")]
fn go_bindings() {
    println!("cargo:rerun-if-changed=../gnark-wrapper/main.go");
    println!("cargo:rerun-if-changed=../gnark-wrapper/go.mod");
    println!("cargo:rerun-if-changed=../gnark-wrapper/go.sum");

    cgo_oligami::Build::new()
        .build_mode(cgo_oligami::BuildMode::CArchive)
        .change_dir("./../gnark-wrapper")
        .package("main.go")
        .build("gnark_wrapper");
}
