#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
mod code {
    include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
}

#[cfg(feature = "std")]
pub use code::WASM_BINARY_OPT as WASM_BINARY;

pub use checkpoint_light_client_io as io;

#[cfg(not(feature = "std"))]
use gstd::{Box, Vec};

#[cfg(not(feature = "std"))]
mod wasm;
