mod committee;

use crate::{crypto, services::Event, utils, State};
use cell::RefCell;
use checkpoint_light_client_io::{
    Error as SyncCommitteeUpdateError, ReplayBack, Slot, SyncCommitteeKeys,
    Update as SyncCommitteeUpdate, MAX_EPOCHS_GAP,
};
use ethereum_common::{
    beacon::{BLSPubKey, BlockHeader as BeaconBlockHeader, SyncAggregate},
    merkle,
    network::Network,
    tree_hash::TreeHash,
    utils as eth_utils, SYNC_COMMITTEE_SIZE,
};
use sails_rs::{prelude::*, rc::Rc};

pub async fn verify(
    network: &Network,
    stored_finalized_slot: Slot,
    stored_sync_committee_current: &SyncCommitteeKeys,
    stored_sync_committee_next: &SyncCommitteeKeys,
    sync_update: SyncCommitteeUpdate,
    sync_aggregate: SyncAggregate,
) -> Result<(Option<BeaconBlockHeader>, Option<Rc<SyncCommitteeKeys>>), SyncCommitteeUpdateError> {
    let SyncCommitteeUpdate {
        signature_slot,
        attested_header,
        finalized_header,
        sync_committee_signature,
        sync_committee_next_aggregate_pubkey,
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
        finality_branch,
    } = sync_update;

    let update_slot_finalized = finalized_header.slot;
    if !(signature_slot > attested_header.slot && attested_header.slot >= update_slot_finalized) {
        return Err(SyncCommitteeUpdateError::InvalidTimestamp);
    }

    let store_period = eth_utils::calculate_period(stored_finalized_slot);
    let update_sig_period = eth_utils::calculate_period(signature_slot);
    let sync_committee = if update_sig_period == store_period + 1 {
        stored_sync_committee_next
    } else if update_sig_period == store_period {
        stored_sync_committee_current
    } else {
        return Err(SyncCommitteeUpdateError::InvalidPeriod);
    };

    let pub_keys =
        utils::get_participating_keys(sync_committee, &sync_aggregate.sync_committee_bits);
    let committee_count = pub_keys.len();

    // committee_count < 512 * 2 / 3
    if committee_count * 3 < SYNC_COMMITTEE_SIZE * 2 {
        return Err(SyncCommitteeUpdateError::LowVoteCount);
    }

    let committee_update = committee::Update::new(
        &attested_header,
        update_slot_finalized,
        sync_committee_next_aggregate_pubkey,
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
    );

    if !crypto::verify_sync_committee_signature(
        network,
        pub_keys,
        &attested_header,
        &sync_committee_signature.0 .0,
        signature_slot,
    )
    .await
    {
        return Err(SyncCommitteeUpdateError::InvalidSignature);
    }

    let mut finalized_header_update = None;
    if update_slot_finalized > stored_finalized_slot {
        if merkle::is_finality_proof_valid(
            network,
            &attested_header,
            &finalized_header,
            &finality_branch,
        ) {
            finalized_header_update = Some(finalized_header);
        } else {
            return Err(SyncCommitteeUpdateError::InvalidFinalityProof);
        }
    }

    let committee_update = match committee_update.verify(network, store_period) {
        Ok(committee_update) => committee_update,
        Err(committee::Error::InvalidNextSyncCommitteeProof) => {
            return Err(SyncCommitteeUpdateError::InvalidNextSyncCommitteeProof)
        }
        Err(committee::Error::InvalidPublicKeys) => {
            return Err(SyncCommitteeUpdateError::InvalidPublicKeys)
        }
    };

    if finalized_header_update.is_none() && committee_update.is_none() {
        return Err(SyncCommitteeUpdateError::NotActual);
    }

    Ok((finalized_header_update, committee_update))
}

pub struct SyncUpdate<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service(events = Event)]
impl<'a> SyncUpdate<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub async fn process(
        &mut self,
        sync_update: SyncCommitteeUpdate,
        sync_aggregate_encoded: Vec<u8>,
    ) -> Result<(), SyncCommitteeUpdateError> {
        let (network, slot, sync_committee_current, sync_committee_next) = {
            let state = self.state.borrow();

            (
                state.network.clone(),
                state.finalized_header.slot,
                Rc::clone(&state.sync_committee_current),
                Rc::clone(&state.sync_committee_next),
            )
        };

        if eth_utils::calculate_epoch(slot) + MAX_EPOCHS_GAP
            <= eth_utils::calculate_epoch(sync_update.finalized_header.slot)
        {
            let state = self.state.borrow();
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

        let sync_aggregate = Decode::decode(&mut &sync_aggregate_encoded[..])
            .map_err(|_| SyncCommitteeUpdateError::InvalidSyncAggregate)?;
        let (finalized_header_update, committee_update) = verify(
            &network,
            slot,
            &sync_committee_current,
            &sync_committee_next,
            sync_update,
            sync_aggregate,
        )
        .await?;

        let mut state = self.state.borrow_mut();

        if let Some(finalized_header) = finalized_header_update {
            state
                .checkpoints
                .push(finalized_header.slot, finalized_header.tree_hash_root());

            self.emit_event(Event::NewCheckpoint {
                slot: finalized_header.slot,
                tree_hash_root: finalized_header.tree_hash_root(),
            })
            .expect("failed to send event");
            state.finalized_header = finalized_header;
        }

        if let Some(sync_committee_next) = committee_update {
            state.sync_committee_current =
                core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
        }

        Ok(())
    }
}
