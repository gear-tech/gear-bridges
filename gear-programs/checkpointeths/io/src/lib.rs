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
    beacon::{BlockHeader as BeaconBlockHeader, Bytes32, SyncAggregate},
    SYNC_COMMITTEE_SIZE,
};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use hex_literal::hex;
use serde::Deserialize;
use ark_serialize::{CanonicalSerialize, CanonicalDeserialize};
pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};

// <G1 as SWCurveConfig>::serialized_size(Compress::No)
pub const G1_UNCOMPRESSED_SIZE: usize = 96;

// <G2 as SWCurveConfig>::serialized_size(Compress::No)
pub const G2_UNCOMPRESSED_SIZE: usize = 192;

pub type ArkScale<T> = ark_scale::ArkScale<T, { ark_scale::HOST_CALL }>;

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash, TypeInfo)]
pub struct SyncCommittee {
    pub pubkeys: FixedArray<BLSPubKey, SYNC_COMMITTEE_SIZE>,
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

#[derive(Debug, Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct G2TypeInfo(pub G2);

impl ark_scale::ArkScaleMaxEncodedLen for G2TypeInfo {
    fn max_encoded_len(_compress: ark_serialize::Compress) -> usize {
        G2_UNCOMPRESSED_SIZE
    }
}

pub type SyncCommitteeKeys = FixedArray<ArkScale<G1TypeInfo>, SYNC_COMMITTEE_SIZE>;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct Init {
    pub genesis: Genesis,
    pub checkpoint: Hash256,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_current_pub_keys: SyncCommitteeKeys,
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
    SyncUpdate(SyncUpdate),
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum HandleResult {
    Checkpoint(CheckpointResult),
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct SyncUpdate {
    pub signature_slot: u64,
    pub attested_header: BeaconBlockHeader,
    pub finalized_header: BeaconBlockHeader,
    pub sync_aggregate: SyncAggregate,
    pub sync_committee_signature: ArkScale<G2TypeInfo>,
    pub sync_committee_next: Option<SyncCommittee>,
    pub sync_committee_next_pub_keys: Option<SyncCommitteeKeys>,
    pub sync_committee_next_branch: Option<Vec<[u8; 32]>>,
    pub finality_branch: Vec<[u8; 32]>,
}
