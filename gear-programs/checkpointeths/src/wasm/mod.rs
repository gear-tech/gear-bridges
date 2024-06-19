use super::*;
use ark_serialize::CanonicalSerialize;
use gstd::{msg, vec};
use io::{
    ethereum_common::{base_types::{Bitvector, FixedArray}, tree_hash::TreeHash, Hash256},
    BeaconBlockHeader, Genesis, Handle, HandleResult, Init, SyncUpdate, G1, G2,
};
use primitive_types::H256;
use state::{Checkpoints, State};

mod committee;
mod crypto;
mod sync_update;
mod utils;

const COUNT: usize = 150_000;
static mut STATE: Option<State<COUNT>> = None;

#[gstd::async_init]
async fn init() {
    let Init {
        genesis,
        sync_committee_current_pub_keys,
        sync_committee_current,
        sync_committee_current_branch,
        update,
    } = msg::load().expect("Unable to decode `Init` message");

    if !utils::check_public_keys(
        &sync_committee_current.pubkeys.0,
        &sync_committee_current_pub_keys,
    ) {
        panic!("Wrong public committee keys");
    }

    let mut finalized_header = update.finalized_header.clone();
    if !merkle::is_current_committee_proof_valid(
        &finalized_header,
        &sync_committee_current,
        &sync_committee_current_branch,
    ) {
        panic!("Current sync committee proof is not valid");
    }

    finalized_header.slot -= 1;
    match sync_update::verify(
        &genesis,
        &finalized_header,
        &sync_committee_current_pub_keys,
        &sync_committee_current_pub_keys,
        update,
    ).await {
        Err(e) => panic!("Failed to verify sync committee update: {e:?}"),

        Ok((Some(finalized_header),
            Some(sync_committee_next),
        )) => unsafe {
                STATE = Some(State {
                    genesis,
                    sync_committee_current: sync_committee_current_pub_keys,
                    sync_committee_next,
                    checkpoints: {
                        let mut checkpoints = Checkpoints::new();
                        checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());
        
                        checkpoints
                    },
                    finalized_header,
                })
            }

        _ => panic!("Incorrect initial sync committee update"),
    }
}

#[gstd::async_main]
async fn main() {
    let state = unsafe { STATE.as_mut() }.expect("The program should be initialized");
    let message: Handle = msg::load().expect("Unable to decode `Handle` message");
    match message {
        Handle::Checkpoint { slot } => {
            let result = state.checkpoints.checkpoint(slot);
            msg::reply(HandleResult::Checkpoint(result), 0)
                .expect("Unable to reply with `HandleResult::Checkpoint`");
        }

        Handle::SyncUpdate(sync_update) => sync_update::handle(state, sync_update).await,
    }
}

#[no_mangle]
extern "C" fn state() {
    let state = unsafe { STATE.as_ref() };
    let checkpoints = state
        .map(|state| state.checkpoints.checkpoints())
        .unwrap_or(vec![]);

    msg::reply(io::meta::State { checkpoints }, 0)
        .expect("Failed to encode or reply with `<AppMetadata as Metadata>::State` from `state()`");
}
