pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ethereum_common::{
    self,
    base_types::FixedArray,
    beacon::{
        BLSPubKey, BlockHeader as BeaconBlockHeader,
    },
    Hash256,
    SYNC_COMMITTEE_SIZE,
};
use sails_rs::prelude::*;
use crate::common::{ReplayBack, Slot};

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

#[derive(Clone, Debug, Decode, TypeInfo)]
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

#[derive(Clone, Debug, Decode, TypeInfo)]
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
    ReplayBackRequired {
        replay_back: Option<ReplayBack>,
        checkpoint: (Slot, Hash256),
    },
}
