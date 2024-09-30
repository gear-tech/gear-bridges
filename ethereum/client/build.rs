use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=*");

    Command::new("forge")
        .arg("build")
        .output()
        .expect("Failed to build solidity code");
}
