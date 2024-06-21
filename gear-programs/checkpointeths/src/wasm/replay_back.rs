use super::*;
use core::cmp::Ordering;
use io::ethereum_common::{EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH};
use gstd::debug;

pub async fn handle_start(
    state: &mut State<COUNT>,
    sync_update: SyncUpdate,
    mut headers: Vec<BeaconBlockHeader>,
) {
    if state.replay_back.is_some() {
        debug!("Already replaying back");
        return;
    }

    let (finalized_header_update, committee_update) = match sync_update::verify(
        &state.genesis,
        &state.finalized_header,
        &state.sync_committee_current,
        &state.sync_committee_next,
        sync_update,
    ).await {
        Err(e) => {
            debug!("sync update verify failed: {e:?}");

            return;
        }

        Ok(result) => result,
    };

    let Some(finalized_header) = finalized_header_update else {
        debug!("Sync update for replay-back should update finalized header");
        return;
    };

    let mut replay_back = ReplayBack {
        finalized_header: finalized_header.clone(),
        sync_committee_next: committee_update,
        checkpoints: {
            let mut checkpoints = Vec::with_capacity(EPOCHS_PER_SYNC_COMMITTEE as usize);
            checkpoints.push((finalized_header.slot, finalized_header.tree_hash_root()));

            checkpoints
        },
        last_header: finalized_header,
    };

    state.replay_back = Some(replay_back);
    if process_headers(state, headers) {
        debug!("Replayd back");
    } else {
        debug!("Started to replay back");
    }
}

pub fn handle(
    state: &mut State<COUNT>,
    headers: Vec<BeaconBlockHeader>,
) {
    if state.replay_back.is_none() {
        debug!("Replaying back wasn't started");
        return;
    }

    if process_headers(state, headers) {
        debug!("Replayed back");
    } else {
        debug!("Continue to replay back");
    }
}

fn process_headers(
    state: &mut State<COUNT>,
    mut headers: Vec<BeaconBlockHeader>,
) -> bool {
    headers.sort_unstable_by(|a, b| {
        if a.slot < b.slot {
            Ordering::Less
        } else if a.slot == b.slot {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    });

    let replay_back = state.replay_back.as_mut().expect("Checked by the caller");
    let (slot_last, checkpoint_last) = state.checkpoints.last().expect("The program initialized so not empty; qed");
    // check blocks hashes
    while let Some(header) = headers.pop() {
        let hash = header.tree_hash_root();
        if hash != replay_back.last_header.parent_root {
            break;
        }

        let slot = header.slot;
        if slot == slot_last && hash == checkpoint_last {
            break;
        }

        replay_back.last_header = header;

        let (slot_next, _) = replay_back.checkpoints.last().expect("At least contains finalized header; qed");
        if slot % SLOTS_PER_EPOCH == 0 || slot + SLOTS_PER_EPOCH < *slot_next {
            replay_back.checkpoints.push((slot, hash));
        }
    }

    if replay_back.last_header.parent_root != checkpoint_last {
        return false;
    }

    // move checkpoints
    while let Some((slot, checkpoint)) = replay_back.checkpoints.pop() {
        state.checkpoints.push(slot, checkpoint);
    }

    if let Some(sync_committee_next) = replay_back.sync_committee_next.take() {
        state.sync_committee_current = core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
    }

    state.finalized_header = replay_back.finalized_header.clone();
    state.replay_back = None;

    true
}
