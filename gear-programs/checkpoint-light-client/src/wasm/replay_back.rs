use super::*;
use core::cmp::Ordering;
use io::{
    ethereum_common::{EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH},
    replay_back::{Error, Status, StatusStart},
};

pub async fn handle_start(
    state: &mut State<STORED_CHECKPOINTS_COUNT>,
    sync_update: SyncCommitteeUpdate,
    mut headers: Vec<BeaconBlockHeader>,
) {
    if state.replay_back.is_some() {
        msg::reply(HandleResult::ReplayBackStart(Err(Error::AlreadyStarted)), 0)
            .expect("Unable to reply with `HandleResult::ReplayBackStart::AlreadyStarted`");
        return;
    }

    let (finalized_header_update, committee_update) = match sync_update::verify(
        &state.network,
        &state.finalized_header,
        &state.sync_committee_current,
        &state.sync_committee_next,
        sync_update,
    )
    .await
    {
        Ok(result) => result,

        Err(e) => {
            msg::reply(HandleResult::ReplayBackStart(Err(Error::Verify(e))), 0)
                .expect("Unable to reply with `HandleResult::ReplayBackStart::Verify`");

            return;
        }
    };

    let Some(finalized_header) = finalized_header_update else {
        msg::reply(
            HandleResult::ReplayBackStart(Err(Error::NoFinalityUpdate)),
            0,
        )
        .expect("Unable to reply with `HandleResult::ReplayBackStart::NoFinalityUpdate`");
        return;
    };

    state.replay_back = Some(ReplayBackState {
        finalized_header: finalized_header.clone(),
        sync_committee_next: committee_update,
        checkpoints: {
            let mut checkpoints = Vec::with_capacity(EPOCHS_PER_SYNC_COMMITTEE as usize);
            checkpoints.push((finalized_header.slot, finalized_header.tree_hash_root()));

            checkpoints
        },
        last_header: finalized_header,
    });
    if process_headers(state, headers) {
        msg::reply(HandleResult::ReplayBackStart(Ok(StatusStart::Finished)), 0)
            .expect("Unable to reply with `HandleResult::ReplayBackStart::Finished`");
    } else {
        msg::reply(
            HandleResult::ReplayBackStart(Ok(StatusStart::InProgress)),
            0,
        )
        .expect("Unable to reply with `HandleResult::ReplayBackStart::Started`");
    }
}

pub fn handle(state: &mut State<STORED_CHECKPOINTS_COUNT>, headers: Vec<BeaconBlockHeader>) {
    if state.replay_back.is_none() {
        msg::reply(HandleResult::ReplayBack(None), 0)
            .expect("Unable to reply with `HandleResult::ReplayBack::None`");
        return;
    }

    if process_headers(state, headers) {
        msg::reply(HandleResult::ReplayBack(Some(Status::Finished)), 0)
            .expect("Unable to reply with `HandleResult::ReplayBack::Finished`");
    } else {
        msg::reply(HandleResult::ReplayBack(Some(Status::InProcess)), 0)
            .expect("Unable to reply with `HandleResult::ReplayBack::InProcess`");
    }
}

fn process_headers(
    state: &mut State<STORED_CHECKPOINTS_COUNT>,
    mut headers: Vec<BeaconBlockHeader>,
) -> bool {
    headers.sort_unstable_by(|a, b| a.slot.cmp(&b.slot));

    let replay_back = state.replay_back.as_mut().expect("Checked by the caller");
    let (slot_last, checkpoint_last) = state
        .checkpoints
        .last()
        .expect("The program initialized so not empty; qed");

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

        let (slot_next, _) = replay_back
            .checkpoints
            .last()
            .expect("At least contains finalized header; qed");
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
        state.sync_committee_current =
            core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
    }

    state.finalized_header = replay_back.finalized_header.clone();
    state.replay_back = None;

    true
}
