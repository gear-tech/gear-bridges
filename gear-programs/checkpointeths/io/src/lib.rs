#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

pub mod meta;

use ethereum_common::{
    base_types::FixedArray,
    beacon::BLSPubKey,
    Hash256,
};
pub use ethereum_common::{
    self,
    tree_hash,
    beacon::{BlockHeader as BeaconBlockHeader, Bytes32},
};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use hex_literal::hex;
use serde::Deserialize;
use ark_serialize::{CanonicalSerialize, CanonicalDeserialize};
pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};

// <G1 as SWCurveConfig>::serialized_size(Compress::No)
pub const G1_UNCOMPRESSED_SIZE: usize = 96;

pub type ArkScale<T> = ark_scale::ArkScale<T, { ark_scale::HOST_CALL }>;

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash, TypeInfo)]
pub struct SyncCommittee {
    pub pubkeys: FixedArray<BLSPubKey, 512>,
    pub aggregate_pubkey: BLSPubKey,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Genesis {
    Mainnet,
    Sepolia,
    Holesky,
}

impl Genesis {
    pub fn hash(&self) -> Hash256 {
        match self {
            Genesis::Mainnet => todo!(),
            Genesis::Sepolia => hex!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078").into(),
            Genesis::Holesky => todo!(),
        }
    }
}

#[derive(Debug, Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct G1TypeInfo(pub G1);

impl ark_scale::ArkScaleMaxEncodedLen for G1TypeInfo {
    fn max_encoded_len(_compress: ark_serialize::Compress) -> usize {
        G1_UNCOMPRESSED_SIZE
    }
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct Init {
    pub genesis: Genesis,
    pub checkpoint: Hash256,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_current_pub_keys: FixedArray<ArkScale<G1TypeInfo>, 512>,
    pub sync_committee_current: SyncCommittee,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Decode, Encode, TypeInfo)]
pub enum CheckpointResult {
    OutDated,
    NotPresent,
    Ok(Hash256),
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Handle {
    Checkpoint {
        slot: u64,
    },
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum HandleResult {
    Checkpoint(CheckpointResult),
}
