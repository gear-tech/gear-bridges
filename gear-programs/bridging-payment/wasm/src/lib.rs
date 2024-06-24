#![no_std]

#[cfg(target_arch = "wasm32")]
pub use bridge_payment_app::wasm::*;
