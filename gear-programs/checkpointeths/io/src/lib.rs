#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use ethereum_common::{
    base_types::FixedArray,
    beacon::BLSPubKey,
    Hash256,
};
pub use ethereum_common::{
    self,
    tree_hash,
    beacon::BlockHeader as BeaconBlockHeader,
};
use hex_literal::hex;
use parity_scale_codec::{Decode, Encode};
use serde::Deserialize;
pub use ark_bls12_381::{G1Projective as G1, G2Projective as G2};

pub type ArkScale<T> = ark_scale::ArkScale<T, { ark_scale::HOST_CALL }>;

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct SyncCommittee {
    pub pubkeys: FixedArray<BLSPubKey, 512>,
    pub aggregate_pubkey: BLSPubKey,
}

#[derive(Debug, Clone, Decode, Encode)]
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

#[derive(Debug, Clone, Decode, Encode)]
pub struct Init {
    pub genesis: Genesis,
    pub checkpoint: Hash256,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_current_pub_keys: ArkScale<Vec<G1>>,
    pub sync_committee_current: SyncCommittee,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
}
