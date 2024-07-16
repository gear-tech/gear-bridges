use super::*;

use gmeta::{In, InOut, Out};
use gstd::prelude::*;

pub struct Metadata;

impl gmeta::Metadata for Metadata {
    type Init = In<Init>;
    type Handle = InOut<Handle, HandleResult>;
    type Others = ();
    type Reply = ();
    type Signal = ();
    type State = Out<State>;
}

#[derive(Debug, Clone, Default, Encode, Decode, TypeInfo)]
pub struct State {
    pub checkpoints: Vec<(Slot, Hash256)>,
    /// The field contains the last processed header if the program is
    /// replaying checkpoints back.
    pub replay_back: Option<BeaconBlockHeader>,
}
