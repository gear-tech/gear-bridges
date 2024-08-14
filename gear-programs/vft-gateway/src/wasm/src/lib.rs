#![no_std]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
include!(concat!(env!("OUT_DIR"), "/vft_gateway_client.rs"));

#[cfg(target_arch = "wasm32")]
pub use vft_gateway_app::wasm::*;
