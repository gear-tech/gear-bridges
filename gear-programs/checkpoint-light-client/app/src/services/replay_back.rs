use sails_rs::prelude::*;
use checkpoint_light_client_io::{ReplayBackError, ReplayBackStatus, Update, BeaconBlockHeader, Error};
use crate::{state::ReplayBackState, State};
use cell::RefCell;
use ethereum_common::{tree_hash::TreeHash, SLOTS_PER_EPOCH, EPOCHS_PER_SYNC_COMMITTEE};

pub struct ReplayBack<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service]
impl<'a> ReplayBack<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub async fn start(&mut self, sync_update: Update,
        sync_aggregate_encoded: Vec<u8>,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError>
    {
        let mut state = self.state.borrow_mut();
        if state.replay_back.is_some() {
            return Err(ReplayBackError::AlreadyStarted);
        }
    
        let sync_aggregate = Decode::decode(&mut &sync_aggregate_encoded[..])
            .map_err(|_| Error::InvalidSyncAggregate)?;
        let (finalized_header_update, committee_update) = super::sync_update::verify(
            &state.network,
            state.finalized_header.slot,
            &state.sync_committee_current,
            &state.sync_committee_next,
            sync_update,
            sync_aggregate,
        )
        .await?;
    
        let finalized_header = finalized_header_update.ok_or(ReplayBackError::NoFinalityUpdate)?;
    
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

        Ok(match process_headers(&mut state, headers) {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }

    pub async fn process(&mut self,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError>
    {
        let mut state = self.state.borrow_mut();
        if state.replay_back.is_none() {
            return Err(ReplayBackError::NotStarted);
        }

        Ok(match process_headers(&mut state, headers) {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }
}

fn process_headers(
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
