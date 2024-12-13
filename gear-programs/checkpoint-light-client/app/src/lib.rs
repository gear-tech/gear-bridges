#![no_std]

mod common;
mod crypto;
mod services;
pub mod state;
pub mod sync_committee;

use cell::RefCell;
use ethereum_common::{
    beacon::BLSPubKey,
    network::Network,
};
use sails_rs::prelude::*;

const STORED_CHECKPOINTS_COUNT: usize = 150_000;

type State = state::State<STORED_CHECKPOINTS_COUNT>;

#[derive(Clone, Debug, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Init {
    pub network: Network,
    pub sync_committee_current_pub_keys: Box<sync_committee::Keys>,
    pub sync_committee_current_aggregate_pubkey: BLSPubKey,
    pub sync_committee_current_branch: Vec<[u8; 32]>,
    pub update: sync_committee::Update,
}

pub struct CheckpointLightClientProgram(RefCell<State>);

#[sails_rs::program]
impl CheckpointLightClientProgram {
    pub fn init(_init: Init) -> Self {
        todo!()
    }

    pub fn checkpoint_for(&self) -> services::CheckpointFor {
        services::CheckpointFor::new(&self.0)
    }

    pub fn state(&self) -> services::State {
        services::State::new(&self.0)
    }
}
