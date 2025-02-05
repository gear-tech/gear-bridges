#![cfg_attr(not(feature = "std"), no_std)]

use gstd::{collections::BTreeMap, prelude::*, BlockNumber};
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

#[derive(Debug, Decode, Encode, TypeInfo, thiserror::Error)]
pub enum Error {
    #[error("Authority set id is not sequential")]
    AuthoritySetIdNotSequential,
    #[error("Two or more proofs submitted at the same block")]
    ManyProofsSubmittedInSameBlock,
}

type AuthoritySetId = u64;

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Proof {
    pub circuit_data: Vec<u8>,
    pub proof: Vec<u8>,
    pub authority_set_id: AuthoritySetId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitMessage {
    pub genesis_proof: Proof,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct HandleMessage {
    pub proof: Vec<u8>,
    pub authority_set_id: AuthoritySetId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct State {
    pub latest_proof: Proof,
    pub proof_blocks: BTreeMap<AuthoritySetId, BlockNumber>,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Reply(Result<(), Error>);
