#![no_std]

use gstd::{collections::BTreeMap, prelude::*, BlockNumber};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

#[cfg(not(feature = "std"))]
mod wasm;

type AuthoritySetId = u64;

#[derive(Debug, Decode, Encode, TypeInfo)]

pub struct InitMessage {
    pub proof: Vec<u8>,
    pub circuit_data: Vec<u8>,
    pub genesis_authority_set_id: AuthoritySetId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct HandleResponse {
    pub proof: Vec<u8>,
    pub authority_set_id: AuthoritySetId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct State {
    pub circuit_data: Vec<u8>,
    pub authority_set_id: AuthoritySetId,
    pub proof_blocks: BTreeMap<AuthoritySetId, BlockNumber>,
}
