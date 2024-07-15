use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=../src");
    println!("cargo::rerun-if-changed=../lib");

    Command::new("forge")
        .arg("build")
        .output()
        .expect("Failed to build solidity code");
}
