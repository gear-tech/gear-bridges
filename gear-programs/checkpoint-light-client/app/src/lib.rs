#![no_std]

mod backend;
mod services;
mod state;

use cell::RefCell;
use checkpoint_light_client_io::{verification::verify_bootstrap, Error, Init};
use ethereum_common::tree_hash::TreeHash;
use sails_rs::prelude::*;

const STORED_CHECKPOINTS_COUNT: usize = 150_000;

type State = state::State<STORED_CHECKPOINTS_COUNT>;

pub struct CheckpointLightClientProgram(RefCell<State>);

#[sails_rs::program]
impl CheckpointLightClientProgram {
    #[export(unwrap_result)]
    pub async fn init(init: Init) -> Result<Self, Error> {
        let verified = verify_bootstrap(init, Syscall::block_timestamp(), &backend::GearBls)
            .await
            .map_err(backend::protocol_error)?;
        Ok(Self(RefCell::new(State {
            network: verified.network,
            sync_committee_current: verified.current_committee.into(),
            sync_committee_next: verified.next_committee.into(),
            checkpoints: {
                let mut checkpoints = state::Checkpoints::new();
                checkpoints
                    .push(
                        verified.finalized_header.slot,
                        verified.finalized_header.tree_hash_root(),
                    )
                    .expect("Initial checkpoint");
                checkpoints
            },
            finalized_header: verified.finalized_header,
            replay_back: None,
            revision: 0,
        })))
    }

    #[export(route = "service_checkpoint_for")]
    pub fn checkpoint_for(&self) -> services::CheckpointFor<'_> {
        services::CheckpointFor::new(&self.0)
    }

    #[export(route = "service_replay_back")]
    pub fn replay_back(&self) -> services::ReplayBack<'_> {
        services::ReplayBack::new(&self.0)
    }

    #[export(route = "service_state")]
    pub fn state(&self) -> services::State<'_> {
        services::State::new(&self.0)
    }

    #[export(route = "service_sync_update")]
    pub fn sync_update(&self) -> services::SyncUpdate<'_> {
        services::SyncUpdate::new(&self.0)
    }
}
