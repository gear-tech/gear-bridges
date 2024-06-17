use io::{Init, G1, G2, BeaconBlockHeader, ethereum_common::{Hash256, base_types::FixedArray, tree_hash::TreeHash}, Handle, HandleResult, SyncUpdate, Genesis};
use gstd::{msg, vec};
use ark_serialize::CanonicalSerialize;
use super::*;
use state::{State, Checkpoints};
use primitive_types::H256;

mod crypto;
mod sync_update;
mod utils;

const COUNT: usize = 150_000;
static mut STATE: Option<State<COUNT>> = None;

#[no_mangle]
extern "C" fn init() {
    let Init {
        genesis,
        checkpoint,
        finalized_header,
        sync_committee_current_pub_keys,
        sync_committee_current,
        sync_committee_current_branch,
    } = msg::load().expect("Unable to decode `Init` message");

    let hash = finalized_header.tree_hash_root();
    if checkpoint != hash {
        panic!("Header hash is not valid. Expected = {checkpoint:?}, actual = {hash:?}");
    }

    if !utils::check_public_keys(&sync_committee_current
            .pubkeys
            .0,
            &sync_committee_current_pub_keys,
        ) {
            panic!("Wrong public committee keys");
        }

    if !merkle::is_current_committee_proof_valid(
        &finalized_header,
        &sync_committee_current,
        &sync_committee_current_branch,
    ) {
        panic!("Current sync committee proof is not valid");
    }

    unsafe {
        STATE = Some(State {
            genesis,
            sync_committee_current: sync_committee_current_pub_keys,
            sync_committee_next: None,
            checkpoints: {
                let mut checkpoints = Checkpoints::new();
                checkpoints.push(finalized_header.slot, checkpoint);

                checkpoints
            },
            finalized_header,
        })
    }
}

#[gstd::async_main]
async fn main() {
    let mut state = unsafe { STATE.as_mut() }.expect("The program should be initialized");
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
extern fn state() {
    let state = unsafe { STATE.as_ref() };
    let checkpoints = state
        .map(|state| state.checkpoints.checkpoints())
        .unwrap_or(vec![]);

    msg::reply(io::meta::State { checkpoints }, 0)
        .expect("Failed to encode or reply with `<AppMetadata as Metadata>::State` from `state()`");
}
