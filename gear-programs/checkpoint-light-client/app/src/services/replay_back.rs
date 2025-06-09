use super::Event;
use crate::{state::ReplayBackState, State};
use cell::RefCell;
use checkpoint_light_client_io::{
    BeaconBlockHeader, Error, ReplayBackError, ReplayBackStatus, Update,
};
use ethereum_common::{tree_hash::TreeHash, EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH};
use sails_rs::{prelude::*, rc::Rc};

pub struct ReplayBack<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service(events = Event)]
impl<'a> ReplayBack<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub async fn start(
        &mut self,
        sync_update: Update,
        sync_aggregate_encoded: Vec<u8>,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError> {
        let (network, slot, sync_committee_current, sync_committee_next) = {
            let state = self.state.borrow();
            if state.replay_back.is_some() {
                return Err(ReplayBackError::AlreadyStarted);
            }

            (
                state.network.clone(),
                state.finalized_header.slot,
                Rc::clone(&state.sync_committee_current),
                Rc::clone(&state.sync_committee_next),
            )
        };

        let sync_aggregate = Decode::decode(&mut &sync_aggregate_encoded[..])
            .map_err(|_| Error::InvalidSyncAggregate)?;
        let (finalized_header_update, committee_update) = super::sync_update::verify(
            &network,
            slot,
            &sync_committee_current,
            &sync_committee_next,
            sync_update,
            sync_aggregate,
        )
        .await?;

        let finalized_header = finalized_header_update.ok_or(ReplayBackError::NoFinalityUpdate)?;
        let mut state = self.state.borrow_mut();
        state.replay_back = Some(ReplayBackState {
            finalized_header: finalized_header.clone(),
            sync_committee_next: committee_update,
            checkpoints: {
                let mut checkpoints = Vec::with_capacity(EPOCHS_PER_SYNC_COMMITTEE as usize);
                checkpoints.push((finalized_header.slot, finalized_header.tree_hash_root()));
                self.emit_event(Event::NewCheckpoint {
                    slot: finalized_header.slot,
                    tree_hash_root: finalized_header.tree_hash_root(),
                })
                .expect("Failed to deposit event");
                checkpoints
            },
            last_header: finalized_header,
        });

        Ok(match process_headers(self, &mut state, headers) {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }

    pub async fn process(
        &mut self,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError> {
        let mut state = self.state.borrow_mut();
        if state.replay_back.is_none() {
            return Err(ReplayBackError::NotStarted);
        }

        Ok(match process_headers(self, &mut state, headers) {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }
}

fn process_headers(
    service: &mut ReplayBack,
    state: &mut State,
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
            service
                .emit_event(Event::NewCheckpoint {
                    slot,
                    tree_hash_root: hash,
                })
                .expect("Failed to deposit event");
        }
    }

    if replay_back.last_header.parent_root != checkpoint_last {
        return false;
    }

    // move checkpoints
    while let Some((slot, checkpoint)) = replay_back.checkpoints.pop() {
        state.checkpoints.push(slot, checkpoint);
        service
            .emit_event(Event::NewCheckpoint {
                slot,
                tree_hash_root: checkpoint,
            })
            .expect("Failed to deposit event");
    }

    if let Some(sync_committee_next) = replay_back.sync_committee_next.take() {
        state.sync_committee_current =
            core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
    }

    state.finalized_header = replay_back.finalized_header.clone();
    state.replay_back = None;

    true
}
