use sails_rs::prelude::*;

pub type Slot = u64;

/// The struct contains slots of the finalized and the last checked headers.
/// This is the state of the checkpoint backfilling process.
#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct ReplayBack {
    pub finalized_header: Slot,
    pub last_header: Slot,
}
