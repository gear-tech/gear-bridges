use super::*;
use ark_serialize::CanonicalSerialize;
use core::{cmp, num::Saturating};
use gstd::{msg, vec};
use io::{
    ethereum_common::{
        base_types::{Bitvector, FixedArray},
        merkle,
        network::Network,
        tree_hash::TreeHash,
        utils as eth_utils, Hash256, SYNC_COMMITTEE_SIZE,
    },
    meta,
    sync_update::Error as SyncCommitteeUpdateError,
    BeaconBlockHeader, Handle, HandleResult, Init, SyncCommitteeKeys, SyncCommitteeUpdate, G1, G2,
    Slot,
};
use primitive_types::H256;
use state::{Checkpoints, ReplayBackState, State};

mod committee;
mod crypto;
mod replay_back;
mod state;
mod sync_update;
mod utils;

const STORED_CHECKPOINTS_COUNT: usize = 150_000;
static mut STATE: Option<State<STORED_CHECKPOINTS_COUNT>> = None;

#[gstd::async_init]
async fn init() {
    let Init {
        network,
        sync_committee_current_pub_keys,
        sync_committee_current_aggregate_pubkey,
        sync_committee_current_branch,
        update,
    } = msg::load().expect("Unable to decode `Init` message");

    let Some(sync_committee_current) = utils::construct_sync_committee(
        sync_committee_current_aggregate_pubkey,
        &sync_committee_current_pub_keys,
    ) else {
        panic!("Wrong public committee keys");
    };

    let mut finalized_header = update.finalized_header.clone();
    if !merkle::is_current_committee_proof_valid(
        &finalized_header,
        &sync_committee_current,
        &sync_committee_current_branch,
    ) {
        panic!("Current sync committee proof is not valid");
    }

    let period = eth_utils::calculate_period(finalized_header.slot) - 1;
    finalized_header.slot = eth_utils::calculate_slot(period);
    match sync_update::verify(
        &network,
        &finalized_header,
        &sync_committee_current_pub_keys,
        &sync_committee_current_pub_keys,
        update,
    )
    .await
    {
        Err(e) => panic!("Failed to verify sync committee update: {e:?}"),

        Ok((Some(finalized_header), Some(sync_committee_next))) => unsafe {
            STATE = Some(State {
                network,
                sync_committee_current: sync_committee_current_pub_keys,
                sync_committee_next,
                checkpoints: {
                    let mut checkpoints = Checkpoints::new();
                    checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());

                    checkpoints
                },
                finalized_header,
                replay_back: None,
            })
        },

        Ok((finalized_header, sync_committee_next)) => panic!(
            "Incorrect initial sync committee update ({}, {})",
            finalized_header.is_some(),
            sync_committee_next.is_some()
        ),
    }
}

#[gstd::async_main]
async fn main() {
    let state = unsafe { STATE.as_mut() }.expect("The program should be initialized");
    let message: Handle = msg::load().expect("Unable to decode `Handle` message");
    match message {
        Handle::GetCheckpointFor { slot } => {
            let result = state.checkpoints.checkpoint(slot);
            msg::reply(HandleResult::Checkpoint(result), 0)
                .expect("Unable to reply with `HandleResult::Checkpoint`");
        }

        Handle::SyncUpdate(sync_update) => sync_update::handle(state, sync_update).await,

        Handle::ReplayBackStart {
            sync_update,
            headers,
        } => replay_back::handle_start(state, sync_update, headers).await,

        Handle::ReplayBack(headers) => replay_back::handle(state, headers),

        Handle::GetState(request) => {
            let reply = utils::construct_state_reply(request, state);
            msg::reply(HandleResult::State(reply), 0)
                .expect("Unable to reply with `HandleResult::State`");
        }
    }
}

#[no_mangle]
extern "C" fn state() {
    let request = msg::load().expect("Unable to decode `StateRequest` message");
    let state = unsafe { STATE.as_ref() }.expect("The program should be initialized");
    let reply = utils::construct_state_reply(request, state);

    msg::reply(reply, 0)
        .expect("Failed to encode or reply with `<AppMetadata as Metadata>::State` from `state()`");
}
