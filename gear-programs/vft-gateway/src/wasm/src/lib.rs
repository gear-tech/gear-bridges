#![no_std]

#[cfg(target_arch = "wasm32")]
pub use vft_gateway::wasm::*;
