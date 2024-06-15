#![cfg_attr(not(feature = "std"), no_std)]

use gstd::{collections::BTreeMap, prelude::*, BlockNumber};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

#[cfg(not(feature = "std"))]
mod wasm;

#[cfg_attr(feature = "std", derive(thiserror::Error))]
#[derive(Debug, Decode, Encode, TypeInfo)]
pub enum Error {
    #[cfg_attr(feature = "std", error("Authority set id is not sequential"))]
    AuthoritySetIdNotSequential,
    #[cfg_attr(
        feature = "std",
        error("Two or more proofs submitted at the same block")
    )]
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
    pub proof: Proof,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct State {
    pub latest_proof: Proof,
    pub proof_blocks: BTreeMap<AuthoritySetId, BlockNumber>,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Reply(Result<(), Error>);
