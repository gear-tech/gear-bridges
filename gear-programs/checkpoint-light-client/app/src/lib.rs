#![no_std]

mod crypto;
mod services;
mod state;
mod utils;

use cell::RefCell;
use checkpoint_light_client_io::Init;
use ethereum_common::{merkle, tree_hash::TreeHash, utils as eth_utils};
use sails_rs::prelude::*;

const STORED_CHECKPOINTS_COUNT: usize = 150_000;

type State = state::State<STORED_CHECKPOINTS_COUNT>;

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
            panic!("Wrong public committee keys for {network:?} network");
        };

        if !merkle::is_current_committee_proof_valid(
            &network,
            &update.finalized_header,
            &sync_committee_current,
            &sync_committee_current_branch,
        ) {
            panic!("Current sync committee proof is not valid for {network:?} network");
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
            Err(e) => panic!("Failed to verify sync committee update for {network:?} network: {e:?}"),

            Ok((Some(finalized_header), Some(sync_committee_next))) => Self(RefCell::new(State {
                network,
                sync_committee_current: sync_committee_current_pub_keys.into(),
                sync_committee_next,
                checkpoints: {
                    let mut checkpoints = state::Checkpoints::new();
                    checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());

                    checkpoints
                },
                finalized_header,
                replay_back: None,
            })),

            Ok((finalized_header, sync_committee_next)) => panic!(
                "Incorrect initial sync committee update for {network:?} network ({}, {})",
                finalized_header.is_some(),
                sync_committee_next.is_some()
            ),
        }
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
