#![cfg_attr(not(feature = "std"), no_std)]

pub mod base_types;
pub mod beacon;
pub mod keccak_hasher;
pub mod memory_db;
pub mod merkle;
pub mod network;
pub mod patricia_trie;
pub mod rlp_node_codec;
pub mod signing_root;
pub mod utils;

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, vec, vec::Vec};
use core::{
    any,
    cmp::PartialEq,
    fmt::{self, Debug},
    marker::PhantomData,
    ops::{Deref, Index, IndexMut},
    slice::{self, SliceIndex},
};

pub use ethereum_types::{H160, H256, U256};
pub use hash_db;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{de, Deserialize};
pub use tree_hash::{self, Hash256};
use tree_hash::{TreeHash, TreeHashType};
pub use trie_db;

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#time-parameters).
pub const SLOTS_PER_EPOCH: u64 = 32;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/beacon-chain.md#sync-committee).
pub const EPOCHS_PER_SYNC_COMMITTEE: u64 = 256;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/beacon-chain.md#sync-committee).
pub const SYNC_COMMITTEE_SIZE: usize = 512;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/light-client/p2p-interface.md#configuration).
pub const MAX_REQUEST_LIGHT_CLIENT_UPDATES: u8 = 128;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/beacon-chain.md#domain-types)
pub const DOMAIN_SYNC_COMMITTEE: [u8; 4] = [0x07, 0x00, 0x00, 0x00];
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#time-parameters-1)
pub const SECONDS_PER_SLOT: u64 = 12;

pub mod electra {
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#execution).
    pub const MAX_DEPOSIT_REQUESTS_PER_PAYLOAD: u64 = 8_192;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#execution).
    pub const MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD: u64 = 16;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#execution).
    pub const MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD: u64 = 2;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#max-operations-per-block).
    pub const MAX_ATTESTER_SLASHINGS: u64 = 1;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#max-operations-per-block).
    pub const MAX_ATTESTATIONS: u64 = 8;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/phase0/beacon-chain.md#misc-1).
    pub const MAX_VALIDATORS_PER_COMMITTEE: u64 = 2_048;
    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/phase0/beacon-chain.md#misc-1).
    pub const MAX_COMMITTEES_PER_SLOT: u64 = 64;
}
