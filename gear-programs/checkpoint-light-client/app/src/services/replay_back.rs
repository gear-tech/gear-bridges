use super::Event;
use crate::{backend, state::ReplayBackState, State};
use cell::RefCell;
use checkpoint_light_client_io::{
    verification::{verify_update, VerificationState},
    BeaconBlockHeader, Error, ReplayBackError, ReplayBackStatus, Update,
};
use ethereum_common::{tree_hash::TreeHash, EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH};
use sails_rs::{prelude::*, rc::Rc};

pub struct ReplayBack<'a> {
    state: &'a RefCell<State>,
}

impl<'a> ReplayBack<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }
}

#[sails_rs::service(events = Event)]
impl<'a> ReplayBack<'a> {
    #[export]
    pub async fn start(
        &mut self,
        sync_update: Update,
        sync_aggregate_encoded: Vec<u8>,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError> {
        let (network, slot, sync_committee_current, sync_committee_next, revision) = {
            let mut state = self.state.borrow_mut();
            if let Some(replay) = &state.replay_back {
                if replay.finalized_header != sync_update.finalized_header {
                    return Err(ReplayBackError::AlreadyStarted);
                }
                return process_headers(self, &mut state, headers).map(|finished| {
                    if finished {
                        ReplayBackStatus::Finished
                    } else {
                        ReplayBackStatus::InProcess
                    }
                });
            }
            if state.finalized_header == sync_update.finalized_header {
                return Ok(ReplayBackStatus::Finished);
            }

            (
                state.network.clone(),
                state.finalized_header.slot,
                Rc::clone(&state.sync_committee_current),
                Rc::clone(&state.sync_committee_next),
                state.revision,
            )
        };

        let verified = verify_update(
            VerificationState {
                network: &network,
                finalized_slot: slot,
                current_committee: &sync_committee_current,
                next_committee: &sync_committee_next,
            },
            Syscall::block_timestamp(),
            sync_update,
            &sync_aggregate_encoded,
            &backend::GearBls,
        )
        .await
        .map_err(backend::protocol_error)?;

        let finalized_header = verified
            .finalized_header
            .ok_or(ReplayBackError::NoFinalityUpdate)?;
        let mut state = self.state.borrow_mut();
        if let Some(replay) = &state.replay_back {
            if replay.finalized_header != finalized_header {
                return Err(ReplayBackError::AlreadyStarted);
            }
            return process_headers(self, &mut state, headers).map(|finished| {
                if finished {
                    ReplayBackStatus::Finished
                } else {
                    ReplayBackStatus::InProcess
                }
            });
        }
        if state.revision != revision {
            if state.finalized_header == finalized_header {
                return Ok(ReplayBackStatus::Finished);
            }
            return Err(Error::StateChanged.into());
        }
        let base_checkpoint = state.checkpoints.last().expect("Initial checkpoint");
        validate_headers(
            &finalized_header,
            base_checkpoint,
            &headers,
            &finalized_header,
        )?;
        let tree_hash_root = finalized_header.tree_hash_root();
        state.replay_back = Some(ReplayBackState {
            base_checkpoint,
            finalized_header: finalized_header.clone(),
            sync_committee_next: verified.next_committee.map(Into::into),
            checkpoints: {
                let mut checkpoints = Vec::with_capacity(EPOCHS_PER_SYNC_COMMITTEE as usize);
                checkpoints.push((finalized_header.slot, tree_hash_root));
                checkpoints
            },
            last_header: finalized_header,
        });
        state.revision = state
            .revision
            .checked_add(1)
            .expect("State revision overflow");

        Ok(match process_headers(self, &mut state, headers)? {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }

    #[export]
    pub async fn process(
        &mut self,
        headers: Vec<BeaconBlockHeader>,
    ) -> Result<ReplayBackStatus, ReplayBackError> {
        let mut state = self.state.borrow_mut();
        if state.replay_back.is_none() {
            return Err(ReplayBackError::NotStarted);
        }

        Ok(match process_headers(self, &mut state, headers)? {
            true => ReplayBackStatus::Finished,
            false => ReplayBackStatus::InProcess,
        })
    }
}

fn process_headers(
    service: &mut ReplayBackExposure<ReplayBack<'_>>,
    state: &mut State,
    mut headers: Vec<BeaconBlockHeader>,
) -> Result<bool, ReplayBackError> {
    let replay_back = state.replay_back.as_mut().expect("Checked by the caller");
    let (slot_last, checkpoint_last) = state
        .checkpoints
        .last()
        .expect("The program initialized so not empty; qed");
    if (slot_last, checkpoint_last) != replay_back.base_checkpoint
        || state.finalized_header.slot != slot_last
    {
        return Err(Error::StateChanged.into());
    }
    validate_headers(
        &replay_back.last_header,
        replay_back.base_checkpoint,
        &headers,
        &replay_back.finalized_header,
    )?;
    // Concurrent callers may include already-consumed ancestry. It has no new effect.
    headers.truncate(headers.partition_point(|header| header.slot < replay_back.last_header.slot));
    let previous_slot = replay_back.last_header.slot;

    // check blocks hashes
    while let Some(header) = headers.pop() {
        let hash = header.tree_hash_root();
        if header.slot == slot_last {
            // This optional terminal header is the immutable authenticated base.
            break;
        }
        let slot = header.slot;
        if hash != replay_back.last_header.parent_root {
            return Err(Error::InvalidHeaders.into());
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
        if replay_back.last_header.slot != previous_slot {
            state.revision = state
                .revision
                .checked_add(1)
                .expect("State revision overflow");
        }
        return Ok(false);
    }

    // move checkpoints
    while let Some((slot, checkpoint)) = replay_back.checkpoints.pop() {
        state
            .checkpoints
            .push(slot, checkpoint)
            .expect("Replay checkpoints must advance from their immutable base");
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
    state.revision = state
        .revision
        .checked_add(1)
        .expect("State revision overflow");

    Ok(true)
}

fn validate_headers(
    last: &BeaconBlockHeader,
    base: (u64, ethereum_common::Hash256),
    headers: &[BeaconBlockHeader],
    target: &BeaconBlockHeader,
) -> Result<(), ReplayBackError> {
    if headers.last().is_some_and(|header| {
        header.slot > target.slot || (header.slot == target.slot && header != target)
    }) {
        return Err(Error::InvalidHeaders.into());
    }
    if headers
        .windows(2)
        .any(|pair| pair[0].slot >= pair[1].slot || pair[1].parent_root != pair[0].tree_hash_root())
    {
        return Err(Error::InvalidHeaders.into());
    }
    let mut child = last;
    for header in headers.iter().rev() {
        if header.slot > last.slot {
            continue;
        }
        if header.slot == last.slot {
            if header.tree_hash_root() != last.tree_hash_root() {
                return Err(Error::InvalidHeaders.into());
            }
            continue;
        }
        if header.slot < base.0
            || header.slot >= child.slot
            || header.tree_hash_root() != child.parent_root
            || (header.slot == base.0 && header.tree_hash_root() != base.1)
        {
            return Err(Error::InvalidHeaders.into());
        }
        child = header;
    }
    Ok(())
}
