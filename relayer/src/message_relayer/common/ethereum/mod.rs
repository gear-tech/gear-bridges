use std::{cmp::Ordering, vec::Drain};

use super::{GearBlockNumber, RelayedMerkleRoot};

pub mod accumulator;
pub mod block_listener;
pub mod block_storage;
pub mod deposit_event_extractor;
pub mod merkle_root_extractor;
pub mod message_paid_event_extractor;
pub mod message_sender;
pub mod status_fetcher;
