use cell::RefCell;
use checkpoint_light_client_io::{IoReplayBack, Slot};
use ethereum_common::Hash256;
use sails_rs::prelude::*;

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct StateData {
    pub checkpoints: Vec<(Slot, Hash256)>,
    /// The field contains the data if the program is
    /// replaying checkpoints back.
    pub replay_back: Option<IoReplayBack>,
}

#[derive(Clone, Debug, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Order {
    Direct,
    Reverse,
}

pub struct State<'a> {
    state: &'a RefCell<crate::State>,
}

#[sails_rs::service]
impl<'a> State<'a> {
    pub fn new(state: &'a RefCell<crate::State>) -> Self {
        Self { state }
    }

    pub fn get(&self, order: Order, index_start: u32, count: u32) -> StateData {
        fn collect<'a, T: 'a + Copy>(
            index_start: u32,
            count: u32,
            iter: impl DoubleEndedIterator<Item = &'a T>,
        ) -> Vec<T> {
            iter.skip(index_start as usize)
                .take(count as usize)
                .copied()
                .collect()
        }

        let state = self.state.borrow();
        let checkpoints = match order {
            Order::Direct => collect(index_start, count, state.checkpoints.iter()),
            Order::Reverse => collect(index_start, count, state.checkpoints.iter().rev()),
        };

        let replay_back = state.replay_back.as_ref().map(|replay_back| IoReplayBack {
            finalized_header: replay_back.finalized_header.slot,
            last_header: replay_back.last_header.slot,
        });

        StateData {
            checkpoints,
            replay_back,
        }
    }
}
