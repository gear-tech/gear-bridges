#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
mod code {
    include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
}

#[cfg(feature = "std")]
pub use code::WASM_BINARY_OPT as WASM_BINARY;

pub use checkpointeths_io as io;

#[cfg(not(feature = "std"))]
use gstd::{Box, Vec};

mod state;

#[cfg(not(feature = "std"))]
mod wasm;

#[cfg(test)]
mod tests;
