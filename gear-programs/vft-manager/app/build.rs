use std::{env, path::PathBuf, process::Command};

fn main() {
    let mut ethereum_contracts_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    ethereum_contracts_dir.pop();
    ethereum_contracts_dir.pop();
    ethereum_contracts_dir.pop();
    ethereum_contracts_dir.push("ethereum");

    println!(
        "cargo::rerun-if-changed={}",
        ethereum_contracts_dir.display()
    );

    Command::new("forge")
        .arg("build")
        .current_dir(ethereum_contracts_dir)
        .output()
        .expect("Failed to build solidity code");
}
