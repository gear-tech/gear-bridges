use super::*;

use gmeta::Out;
use gstd::prelude::*;

pub struct Metadata;

impl gmeta::Metadata for Metadata {
    type Init = ();
    type Handle = ();
    type Others = ();
    type Reply = ();
    type Signal = ();
    type State = Out<State>;
}

#[derive(Debug, Clone, Default, Encode, Decode, TypeInfo)]
pub struct State {
    pub checkpoints: Vec<(u64, Hash256)>,
}
