use io::{Init, G1, ethereum_common::{base_types::FixedArray, tree_hash::TreeHash}, Handle, HandleResult, SyncUpdate};
use gstd::{msg, vec};
use ark_serialize::CanonicalSerialize;
use super::*;
use state::{State, Checkpoints};

mod sync_update;

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

    // check that provided public keys belong to the committee
    let mut pub_keys = Vec::with_capacity(sync_committee_current
        .pubkeys
        .0.len());
    let mut buffer = Vec::with_capacity(100);
    for (pub_key_compressed, pub_key) in sync_committee_current
        .pubkeys
        .0
        .as_ref()
        .iter()
        .zip(sync_committee_current_pub_keys.0.iter())
    {
        buffer.clear();

        assert!(
            matches!(
                <G1 as CanonicalSerialize>::serialize_compressed(&pub_key.0.0, &mut buffer),
                Ok(_) if pub_key_compressed.as_ref() == &buffer[..],
            )
        );

        pub_keys.push(pub_key.0.0);
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
            sync_committee_current: FixedArray(pub_keys.try_into().expect("array of public keys has the right size; qed")),
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
