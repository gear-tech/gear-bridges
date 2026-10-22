#![no_std]

mod crypto;
mod services;
mod state;
mod utils;

use cell::RefCell;
use checkpoint_light_client_io::{Error, Init};
use ethereum_common::{merkle, tree_hash::TreeHash, utils as eth_utils};
use sails_rs::prelude::*;

const STORED_CHECKPOINTS_COUNT: usize = 150_000;

type State = state::State<STORED_CHECKPOINTS_COUNT>;

pub struct CheckpointLightClientProgram(RefCell<State>);

#[sails_rs::program]
impl CheckpointLightClientProgram {
    #[export(unwrap_result)]
    pub async fn init(init: Init) -> Result<Self, Error> {
        if init.bootstrap_header.tree_hash_root() != init.trusted_bootstrap_root {
            return Err(Error::InvalidBootstrapRoot);
        }
        let period = eth_utils::calculate_period(init.update.finalized_header.slot)
            .checked_sub(1)
            .ok_or(Error::UnsupportedBootstrapPeriod)?;
        let Init {
            network,
            bootstrap_header,
            sync_committee_current_pub_keys,
            sync_committee_current_aggregate_pubkey,
            sync_committee_current_branch,
            update,
            sync_aggregate_encoded,
            trusted_bootstrap_root: _,
        } = init;

        let sync_aggregate = <ethereum_common::beacon::SyncAggregate as sails_rs::scale_codec::DecodeAll>::decode_all(&mut &sync_aggregate_encoded[..])
            .map_err(|_| Error::InvalidSyncAggregate)?;

        let Some(sync_committee_current) = utils::construct_sync_committee(
            sync_committee_current_aggregate_pubkey,
            &sync_committee_current_pub_keys,
        ) else {
            return Err(Error::InvalidPublicKeys);
        };

        if !(bootstrap_header.slot <= update.finalized_header.slot
            && eth_utils::calculate_period(bootstrap_header.slot)
                == eth_utils::calculate_period(update.finalized_header.slot))
        {
            return Err(Error::InvalidBootstrapUpdate);
        }

        if !merkle::is_current_committee_proof_valid(
            &network,
            &bootstrap_header,
            &sync_committee_current,
            &sync_committee_current_branch,
        ) {
            return Err(Error::InvalidBootstrapProof);
        }

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
            Err(e) => Err(e),

            Ok((Some(finalized_header), Some(sync_committee_next))) => {
                Ok(Self(RefCell::new(State {
                    network,
                    sync_committee_current: sync_committee_current_pub_keys.into(),
                    sync_committee_next,
                    checkpoints: {
                        let mut checkpoints = state::Checkpoints::new();
                        checkpoints
                            .push(finalized_header.slot, finalized_header.tree_hash_root())
                            .expect("Initial checkpoint");

                        checkpoints
                    },
                    finalized_header,
                    replay_back: None,
                    revision: 0,
                })))
            }

            Ok(_) => Err(Error::InvalidBootstrapUpdate),
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
