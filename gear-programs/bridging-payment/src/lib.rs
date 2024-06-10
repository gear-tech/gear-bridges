#![no_std]

use gstd::ActorId;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

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
    ReclaimFees,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct State {
    pub fee: u128,
}
