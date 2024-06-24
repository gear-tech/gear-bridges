#![no_std]

#[cfg(target_arch = "wasm32")]
pub use grc20_gateway_app::wasm::*;
