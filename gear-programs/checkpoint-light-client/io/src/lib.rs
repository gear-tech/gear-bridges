#![cfg_attr(not(feature = "std"), no_std)]

pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
pub use ethereum_common::{
    self,
    base_types::FixedArray,
    beacon::{BLSPubKey, BlockHeader as BeaconBlockHeader},
    network::Network,
    Hash256, SYNC_COMMITTEE_SIZE,
};
use sails_rs::prelude::*;

pub type Slot = u64;

/// The struct contains slots of the finalized and the last checked headers.
/// This is the state of the checkpoint backfilling process.
#[derive(Clone, Debug, Decode, Encode, PartialEq, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct IoReplayBack {
    pub finalized_header: Slot,
    pub last_header: Slot,
}

// The constant defines how many epochs may be skipped.
pub const MAX_EPOCHS_GAP: u64 = 3;
// <G1 as SWCurveConfig>::serialized_size(Compress::No)
pub const G1_UNCOMPRESSED_SIZE: usize = 96;
// <G2 as SWCurveConfig>::serialized_size(Compress::No)
pub const G2_UNCOMPRESSED_SIZE: usize = 192;

pub type ArkScale<T> = ark_scale::ArkScale<T, { ark_scale::HOST_CALL }>;

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

pub type Keys = FixedArray<ArkScale<G1TypeInfo>, SYNC_COMMITTEE_SIZE>;

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Update {
    pub signature_slot: u64,
    pub attested_header: BeaconBlockHeader,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_signature: ArkScale<G2TypeInfo>,
    pub sync_committee_next_aggregate_pubkey: Option<BLSPubKey>,
    pub sync_committee_next_pub_keys: Option<Box<Keys>>,
    pub sync_committee_next_branch: Option<Vec<[u8; 32]>>,
    pub finality_branch: Vec<[u8; 32]>,
}

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Init {
    pub network: Network,
    pub sync_committee_current_pub_keys: Box<Keys>,
    pub sync_committee_current_aggregate_pubkey: BLSPubKey,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
    pub update: Update,
    pub sync_aggregate_encoded: Vec<u8>,
}

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Error {
    InvalidTimestamp,
    InvalidPeriod,
    LowVoteCount,
    NotActual,
    InvalidSignature,
    InvalidFinalityProof,
    InvalidNextSyncCommitteeProof,
    InvalidPublicKeys,
    InvalidSyncAggregate,
    ReplayBackRequired {
        replay_back: Option<IoReplayBack>,
        checkpoint: (Slot, Hash256),
    },
}

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ReplayBackStatus {
    InProcess,
    Finished,
}

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ReplayBackError {
    AlreadyStarted,
    NotStarted,
    Verify(Error),
    NoFinalityUpdate,
}

impl From<Error> for ReplayBackError {
    fn from(e: Error) -> Self {
        ReplayBackError::Verify(e)
    }
}
