#![no_std]

#[cfg(target_arch = "wasm32")]
pub use bridging_payment::wasm::*;
