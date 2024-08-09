use super::*;

use gmeta::{In, InOut};
use gstd::prelude::*;

pub struct Metadata;

impl gmeta::Metadata for Metadata {
    type Init = In<Init>;
    type Handle = InOut<Handle, HandleResult>;
    type Others = ();
    type Reply = ();
    type Signal = ();
    type State = InOut<StateRequest, State>;
}

#[derive(Debug, Clone, Default, Encode, Decode, TypeInfo)]
pub struct State {
    pub checkpoints: Vec<(Slot, Hash256)>,
    /// The field contains the data if the program is
    /// replaying checkpoints back.
    pub replay_back: Option<ReplayBack>,
}

/// The struct contains slots of the finalized and the last checked headers.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct ReplayBack {
    pub finalized_header: Slot,
    pub last_header: Slot,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Order {
    Direct,
    Reverse,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StateRequest {
    // The flag specifies the subslice position
    pub order: Order,
    // Parameters determine checkpoints count in the response
    pub index_start: u32,
    pub count: u32,
}
