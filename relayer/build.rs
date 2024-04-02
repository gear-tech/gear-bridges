fn main() {
    cgo_oligami::Build::new()
        .build_mode(cgo_oligami::BuildMode::CArchive)
        .change_dir("./../gnark-wrapper")
        .package("main.go")
        .build("gnark_wrapper");

    println!("cargo:rerun-if-changed=../gnark-wrapper/main.go");
}
