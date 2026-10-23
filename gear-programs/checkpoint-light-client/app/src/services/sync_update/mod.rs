use crate::{backend, services::Event, State};
use cell::RefCell;
use checkpoint_light_client_io::{
    verification::{verify_update, VerificationState},
    Error as SyncCommitteeUpdateError, ReplayBack, Update as SyncCommitteeUpdate, MAX_EPOCHS_GAP,
};
use ethereum_common::{tree_hash::TreeHash, utils as eth_utils};
use sails_rs::{prelude::*, rc::Rc};

pub struct SyncUpdate<'a> {
    state: &'a RefCell<State>,
}

impl<'a> SyncUpdate<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }
}

#[sails_rs::service(events = Event)]
impl<'a> SyncUpdate<'a> {
    #[export]
    pub async fn process(
        &mut self,
        sync_update: SyncCommitteeUpdate,
        sync_aggregate_encoded: Vec<u8>,
    ) -> Result<(), SyncCommitteeUpdateError> {
        let (network, slot, sync_committee_current, sync_committee_next, revision) = {
            let state = self.state.borrow();

            if state.replay_back.is_some()
                || eth_utils::calculate_epoch(state.finalized_header.slot) + MAX_EPOCHS_GAP
                    <= eth_utils::calculate_epoch(sync_update.finalized_header.slot)
            {
                return Err(SyncCommitteeUpdateError::ReplayBackRequired {
                    replay_back: state.replay_back.as_ref().map(|replay_back| ReplayBack {
                        finalized_header: replay_back.finalized_header.slot,
                        last_header: replay_back.last_header.slot,
                    }),
                    checkpoint: state
                        .checkpoints
                        .last()
                        .expect("The program should be initialized so there is a checkpoint"),
                });
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

        let mut state = self.state.borrow_mut();
        if state.revision != revision {
            return Err(SyncCommitteeUpdateError::StateChanged);
        }

        if let Some(finalized_header) = verified.finalized_header {
            let tree_hash_root = finalized_header.tree_hash_root();
            state
                .checkpoints
                .push(finalized_header.slot, tree_hash_root)
                .map_err(|_| SyncCommitteeUpdateError::NotActual)?;

            self.emit_event(Event::NewCheckpoint {
                slot: finalized_header.slot,
                tree_hash_root,
            })
            .expect("failed to send event");
            state.finalized_header = finalized_header;
        }

        if let Some(sync_committee_next) = verified.next_committee {
            state.sync_committee_current =
                core::mem::replace(&mut state.sync_committee_next, sync_committee_next.into());
        }
        state.revision = state
            .revision
            .checked_add(1)
            .expect("State revision overflow");

        Ok(())
    }
}
