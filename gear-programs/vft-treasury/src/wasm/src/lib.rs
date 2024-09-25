#![no_std]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
include!(concat!(env!("OUT_DIR"), "/vft_treasury_client.rs"));

#[cfg(target_arch = "wasm32")]
pub use vft_treasury_app::wasm::*;
