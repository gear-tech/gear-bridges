use crate::{state::CheckpointError, State};
use cell::RefCell;
use checkpoint_light_client_io::Slot;
use ethereum_common::Hash256;
use sails_rs::prelude::*;

pub struct CheckpointFor<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service]
impl<'a> CheckpointFor<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub fn get(&self, slot: Slot) -> Result<(Slot, Hash256), CheckpointError> {
        let state = self.state.borrow();

        state.checkpoints.checkpoint(slot)
    }
}
