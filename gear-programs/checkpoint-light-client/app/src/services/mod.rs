mod checkpoint;
mod replay_back;
mod state;
pub mod sync_update;

pub use checkpoint::CheckpointFor;
use ethereum_common::Hash256;
pub use replay_back::ReplayBack;
use sails_rs::{Encode, TypeInfo};
pub use state::State;
pub use sync_update::SyncUpdate;

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Event {
    NewCheckpoint { slot: u64, tree_hash_root: Hash256 },
}
