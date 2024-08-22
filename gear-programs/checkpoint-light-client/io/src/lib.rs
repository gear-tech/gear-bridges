#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

pub mod meta;
pub mod replay_back;
pub mod sync_update;

pub use sync_update::SyncCommitteeUpdate;

pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
pub use ethereum_common::{
    self,
    beacon::{
        BLSPubKey, Block as BeaconBlock, BlockHeader as BeaconBlockHeader, Bytes32, SyncAggregate,
    },
    network::Network,
    tree_hash, SYNC_COMMITTEE_SIZE,
};
use ethereum_common::{base_types::FixedArray, Hash256};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

// <G1 as SWCurveConfig>::serialized_size(Compress::No)
pub const G1_UNCOMPRESSED_SIZE: usize = 96;

// <G2 as SWCurveConfig>::serialized_size(Compress::No)
pub const G2_UNCOMPRESSED_SIZE: usize = 192;

pub type ArkScale<T> = ark_scale::ArkScale<T, { ark_scale::HOST_CALL }>;

pub type Slot = u64;

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
    pub network: Network,
    pub sync_committee_current_pub_keys: Box<SyncCommitteeKeys>,
    pub sync_committee_current_aggregate_pubkey: BLSPubKey,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
    pub update: SyncCommitteeUpdate,
}

#[derive(Debug, Clone, Decode, Encode, TypeInfo)]
pub enum CheckpointError {
    OutDated,
    NotPresent,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Handle {
    GetCheckpointFor {
        slot: Slot,
    },
    SyncUpdate(SyncCommitteeUpdate),
    ReplayBackStart {
        sync_update: SyncCommitteeUpdate,
        headers: Vec<BeaconBlockHeader>,
    },
    ReplayBack(Vec<BeaconBlockHeader>),
    GetState(meta::StateRequest),
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum HandleResult {
    Checkpoint(Result<(Slot, Hash256), CheckpointError>),
    SyncUpdate(Result<(), sync_update::Error>),
    ReplayBackStart(Result<replay_back::StatusStart, replay_back::Error>),
    ReplayBack(Option<replay_back::Status>),
    State(meta::State),
}
