use checkpointeths_io::{self as io, Init, G1, ethereum_common::tree_hash::TreeHash};
use gstd::msg;
use ark_serialize::CanonicalSerialize;
use super::*;
use state::{State, Checkpoints};

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
    let mut buffer = Vec::with_capacity(512);
    let pub_key_count = sync_committee_current
        .pubkeys
        .0
        .as_ref()
        .iter()
        .zip(sync_committee_current_pub_keys.0.iter())
        .fold(0, |count, (pub_key_compressed, pub_key)| {
            buffer.clear();

            match <G1 as CanonicalSerialize>::serialize_compressed(&pub_key, &mut buffer) {
                Ok(_) => {
                    assert_eq!(pub_key_compressed.as_ref(), &buffer[..]);

                    count + 1
                }

                Err(_) => count,
            }
        });
    assert_eq!(pub_key_count, 512);

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
            sync_committee_current: sync_committee_current_pub_keys.0,
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
}
