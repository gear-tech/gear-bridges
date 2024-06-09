#![no_std]

use gstd::ActorId;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

#[cfg(feature = "std")]
mod code {
    include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
}

#[cfg(feature = "std")]
pub use code::WASM_BINARY_OPT as WASM_BINARY;

#[cfg(not(feature = "std"))]
mod wasm;

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitMessage {
    pub grc20_gateway: ActorId,
    pub fee: u128,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub enum AdminMessage {
    SetFee(u128),
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct State {
    pub fee: u128,
}
