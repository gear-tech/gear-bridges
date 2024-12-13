#![no_std]

pub mod sync_committee;

use ethereum_common::{
    beacon::BLSPubKey,
    network::Network,
};
use sails_rs::prelude::*;

struct CheckpointLightClientService(());

#[sails_rs::service]
impl CheckpointLightClientService {
    pub fn new() -> Self {
        Self(())
    }

    // Service's method (command)
    pub fn do_something(&mut self) -> String {
        "Hello from CheckpointLightClient!".to_string()
    }

    // Service's query
    pub fn get_something(&self) -> String {
        "Hello from CheckpointLightClient!".to_string()
    }    
}

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

pub struct CheckpointLightClientProgram(());

#[sails_rs::program]
impl CheckpointLightClientProgram {
    pub fn init(_init: Init) -> Self {
        Self(())
    }

    // Exposed service
    pub fn checkpoint_light_client(&self) -> CheckpointLightClientService {
        CheckpointLightClientService::new()
    }
}
