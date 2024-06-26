#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

pub mod meta;
pub mod replay_back;
pub mod sync_update;

pub use sync_update::SyncUpdate;

pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
pub use ethereum_common::{
    self,
    beacon::{BlockHeader as BeaconBlockHeader, Bytes32, SyncAggregate},
    tree_hash, SYNC_COMMITTEE_SIZE,
};
use ethereum_common::{base_types::FixedArray, beacon::BLSPubKey, Hash256};
use hex_literal::hex;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::Deserialize;

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
    pub fn validators_root(&self) -> Hash256 {
        use Genesis::*;

        match self {
            Mainnet => hex!("4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"),
            Sepolia => hex!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            Holesky => hex!("9143aa7c615a7f7115e2b6aac319c03529df8242ae705fba9df39b79c59fa8b1"),
        }.into()
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
    pub sync_committee_current_pub_keys: Box<SyncCommitteeKeys>,
    pub sync_committee_current: SyncCommittee,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
    pub update: SyncUpdate,
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
    ReplayBackStart {
        sync_update: SyncUpdate,
        headers: Vec<BeaconBlockHeader>,
    },
    ReplayBack(Vec<BeaconBlockHeader>),
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum HandleResult {
    Checkpoint(CheckpointResult),
    SyncUpdate(Result<(), sync_update::Error>),
    ReplayBackStart(Result<replay_back::StatusStart, replay_back::Error>),
    ReplayBack(Option<replay_back::Status>),
}
