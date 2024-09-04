use sails_client_gen::ClientGenerator;
use std::{env, path::PathBuf};

fn main() {
    go_bindings();
    bridging_payment_client();
}

fn go_bindings() {
    println!("cargo:rerun-if-changed=../gnark-wrapper/main.go");

    cgo_oligami::Build::new()
        .build_mode(cgo_oligami::BuildMode::CArchive)
        .change_dir("./../gnark-wrapper")
        .package("main.go")
        .build("gnark_wrapper");
}

fn bridging_payment_client() {
    println!(
        "cargo:rerun-if-changed=../gear-programs/bridging-payment/src/wasm/bridging-payment.idl"
    );

    let out_dir_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let idl_file_path =
        PathBuf::from("../gear-programs/bridging-payment/src/wasm/bridging-payment.idl");
    let client_rs_file_path = out_dir_path.join("bridging_payment_client.rs");

    ClientGenerator::from_idl_path(&idl_file_path)
        .generate_to(client_rs_file_path)
        .unwrap();
}
