use super::*;
use committee::{Error as CommitteeError, Update as CommitteeUpdate};
use gstd::debug;

pub async fn handle(state: &mut State<STORED_CHECKPOINTS_COUNT>, sync_update: SyncCommitteeUpdate) {
    let (finalized_header_update, committee_update) = match verify(
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
            let result = HandleResult::SyncUpdate(Err(e));
            msg::reply(result, 0).expect("Unable to reply with `HandleResult::SyncUpdate::Error`");

            return;
        }
    };

    if let Some(finalized_header) = finalized_header_update {
        if eth_utils::calculate_epoch(state.finalized_header.slot) + io::sync_update::MAX_EPOCHS_GAP
            <= eth_utils::calculate_epoch(finalized_header.slot)
        {
            let result =
                HandleResult::SyncUpdate(Err(io::sync_update::Error::ReplayBackRequired {
                    replayed_slot: state
                        .replay_back
                        .as_ref()
                        .map(|replay_back| replay_back.last_header.slot),
                    checkpoint: state
                        .checkpoints
                        .last()
                        .expect("The program should be initialized so there is a checkpoint"),
                }));
            msg::reply(result, 0).expect("Unable to reply with `HandleResult::SyncUpdate::Error`");

            return;
        }

        state
            .checkpoints
            .push(finalized_header.slot, finalized_header.tree_hash_root());
        state.finalized_header = finalized_header;
    }

    if let Some(sync_committee_next) = committee_update {
        state.sync_committee_current =
            core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
    }

    msg::reply(HandleResult::SyncUpdate(Ok(())), 0)
        .expect("Unable to reply with `HandleResult::SyncUpdate::Ok`");
}

pub async fn verify(
    network: &Network,
    stored_finalized_header: &BeaconBlockHeader,
    stored_sync_committee_current: &SyncCommitteeKeys,
    stored_sync_committee_next: &SyncCommitteeKeys,
    sync_update: SyncCommitteeUpdate,
) -> Result<(Option<BeaconBlockHeader>, Option<Box<SyncCommitteeKeys>>), SyncCommitteeUpdateError> {
    let SyncCommitteeUpdate {
        signature_slot,
        attested_header,
        finalized_header,
        sync_aggregate,
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

    let store_period = eth_utils::calculate_period(stored_finalized_header.slot);
    let update_sig_period = eth_utils::calculate_period(signature_slot);
    let sync_committee = if update_sig_period == store_period + 1 {
        stored_sync_committee_next
    } else if update_sig_period == store_period {
        stored_sync_committee_current
    } else {
        return Err(SyncCommitteeUpdateError::InvalidPeriod);
    };

    let pub_keys =
        utils::get_participating_keys(&sync_committee, &sync_aggregate.sync_committee_bits);
    let committee_count = pub_keys.len();

    // committee_count < 512 * 2 / 3
    if committee_count * 3 < SYNC_COMMITTEE_SIZE * 2 {
        return Err(SyncCommitteeUpdateError::LowVoteCount);
    }

    let committee_update = CommitteeUpdate::new(
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
    if update_slot_finalized > stored_finalized_header.slot {
        if merkle::is_finality_proof_valid(&attested_header, &finalized_header, &finality_branch) {
            finalized_header_update = Some(finalized_header);
        } else {
            return Err(SyncCommitteeUpdateError::InvalidFinalityProof);
        }
    }

    let committee_update = match committee_update.verify(store_period) {
        Ok(committee_update) => committee_update,
        Err(CommitteeError::InvalidNextSyncCommitteeProof) => {
            return Err(SyncCommitteeUpdateError::InvalidNextSyncCommitteeProof)
        }
        Err(CommitteeError::InvalidPublicKeys) => {
            return Err(SyncCommitteeUpdateError::InvalidPublicKeys)
        }
    };

    if finalized_header_update.is_none() && committee_update.is_none() {
        return Err(SyncCommitteeUpdateError::NotActual);
    }

    Ok((finalized_header_update, committee_update))
}
