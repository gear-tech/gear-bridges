#![cfg_attr(not(feature = "std"), no_std)]

pub mod base_types;
pub mod beacon;
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

pub use ethereum_types::U256;
use parity_scale_codec::{Decode, Encode};
use serde::{de, Deserialize};
pub use tree_hash::{self, Hash256};
use tree_hash::{TreeHash, TreeHashType};

pub const SLOTS_PER_EPOCH: u64 = 32;
pub const EPOCHS_PER_SYNC_COMMITTEE: u64 = 256;
