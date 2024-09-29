#![cfg_attr(not(feature = "std"), no_std)]

pub mod base_types;
pub mod beacon;
pub mod keccak_hasher;
pub mod memory_db;
pub mod merkle;
pub mod network;
pub mod patricia_trie;
pub mod rlp_node_codec;
pub mod utils;

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, vec, vec::Vec};
use core::{
    any,
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

pub const SLOTS_PER_EPOCH: u64 = 32;
pub const EPOCHS_PER_SYNC_COMMITTEE: u64 = 256;
pub const SYNC_COMMITTEE_SIZE: usize = 512;
