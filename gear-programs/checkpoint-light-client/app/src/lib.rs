#![no_std]

mod common;
mod crypto;
mod services;
mod state;
mod sync_committee;
mod utils;

use cell::RefCell;
use ethereum_common::{
    beacon::BLSPubKey,
    network::Network,
    merkle,
    utils as eth_utils,
    tree_hash::TreeHash,
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
    pub sync_aggregate_encoded: Vec<u8>,
}

pub struct CheckpointLightClientProgram(RefCell<State>);

#[sails_rs::program]
impl CheckpointLightClientProgram {
    pub async fn init(init: Init) -> Self {
        let Init {
            network,
            sync_committee_current_pub_keys,
            sync_committee_current_aggregate_pubkey,
            sync_committee_current_branch,
            update,
            sync_aggregate_encoded,
        } = init;

        let sync_aggregate = Decode::decode(&mut &sync_aggregate_encoded[..])
            .expect("Correctly scale-encoded SyncAggregate");

        let Some(sync_committee_current) = utils::construct_sync_committee(
            sync_committee_current_aggregate_pubkey,
            &sync_committee_current_pub_keys,
        ) else {
            panic!("Wrong public committee keys");
        };
    
        if !merkle::is_current_committee_proof_valid(
            &update.finalized_header,
            &sync_committee_current,
            &sync_committee_current_branch,
        ) {
            panic!("Current sync committee proof is not valid");
        }
    
        let period = eth_utils::calculate_period(update.finalized_header.slot) - 1;
        match services::sync_update::verify(
            &network,
            eth_utils::calculate_slot(period),
            &sync_committee_current_pub_keys,
            &sync_committee_current_pub_keys,
            update,
            sync_aggregate,
        )
        .await
        {
            Err(e) => panic!("Failed to verify sync committee update: {e:?}"),
    
            Ok((Some(finalized_header), Some(sync_committee_next))) => {
                Self(RefCell::new(State {
                    network,
                    sync_committee_current: sync_committee_current_pub_keys,
                    sync_committee_next,
                    checkpoints: {
                        let mut checkpoints = state::Checkpoints::new();
                        checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());
    
                        checkpoints
                    },
                    finalized_header,
                    replay_back: None,
                }))
            },
    
            Ok((finalized_header, sync_committee_next)) => panic!(
                "Incorrect initial sync committee update ({}, {})",
                finalized_header.is_some(),
                sync_committee_next.is_some()
            ),
        }
    }

    pub fn checkpoint_for(&self) -> services::CheckpointFor {
        services::CheckpointFor::new(&self.0)
    }

    pub fn state(&self) -> services::State {
        services::State::new(&self.0)
    }
}
