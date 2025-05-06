{
  description = "Flake providing development environment for Gear Bridge";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, flake-utils, nixpkgs, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in {
        toolchain.extensions = [
          "rust-src"
          "rust-analyzer"
          "llvm-tools"
        ];
        toolchain.targets = [
          "wasm32v1-none"
          "wasm32-unknown-unknown"
        ];

        devShells.default = with pkgs; mkShell {
          CRATE_CC_NO_DEFAULTS = "1"; 
          hardeningDisable = [ "fortify" "zerocallusedregs" "stackprotector" ]; 
          IN_NIX_SHELL = "flake"; # required to build gear
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          
          # required for rustup to work.
          shellHook = ''
            export PATH=$PATH:''${CARGO_HOME:-~/.cargo}/bin
          '';

          packages = [        
            protobuf
            rocksdb
            gcc
            go
            llvmPackages.clang
            llvmPackages.libclang
            jemalloc
            binaryen
            foundry
            toolchain
            rustup # demos from gear use rustup, enable it.
            cmake
            git
            openssl
            pkg-config
          ];
        };
      }
    );
}
