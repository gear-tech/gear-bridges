use super::*;
use gstd::debug;
use io::{
    SyncCommitteeKeys,
    ethereum_common::{utils as eth_utils, SYNC_COMMITTEE_SIZE, base_types::Bitvector},
};

pub async fn handle(state: &mut State<COUNT>, sync_update: SyncUpdate) {
    let SyncUpdate {
        signature_slot,
        attested_header,
        finalized_header,
        sync_aggregate,
        sync_committee_signature,
        sync_committee_next,
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
        finality_branch,
    } = sync_update;

    let update_finalized_slot = finalized_header
        .slot;
    let valid_time = signature_slot > attested_header.slot
        && attested_header.slot >= update_finalized_slot;
    if !valid_time {
        debug!("ConsensusError::InvalidTimestamp.into()");
        return;
    }

    let store_period = eth_utils::calculate_period(state.finalized_header.slot);
    let update_sig_period = eth_utils::calculate_period(signature_slot);
    let sync_committee = match state.sync_committee_next.as_ref() {
        Some(sync_committee) if update_sig_period == store_period + 1 => sync_committee,

        _ if update_sig_period == store_period => &state.sync_committee_current,

        _ => {
            debug!("ConsensusError::InvalidPeriod.into()");
            return;
        }
    };

    let pub_keys =
        get_participating_keys(&sync_committee, &sync_aggregate.sync_committee_bits);
    let committee_count = pub_keys.len();

    // committee_count < 512 * 2 / 3
    if committee_count * 3 < SYNC_COMMITTEE_SIZE * 2 {
        debug!("skipping block with low vote count");
        return;
    }

    let update_finalized_period = eth_utils::calculate_period(update_finalized_slot);
    let update_is_newer = update_finalized_slot > state.finalized_header.slot;
    let update_attested_period = eth_utils::calculate_period(attested_header.slot);
    let update_has_finalized_next_committee =
        // has sync update
        sync_committee_next_pub_keys.is_some() && sync_committee_next_branch.is_some()
        && update_finalized_period == update_attested_period;

    debug!("store_period = {store_period}, update_sig_period = {update_sig_period}, update_finalized_period = {update_finalized_period}, update_attested_period = {update_attested_period}");

    if update_is_newer || (update_has_finalized_next_committee && state.sync_committee_next.is_none()) {
        let is_valid_sig = crypto::verify_sync_committee_signature(
            &state.genesis,
            pub_keys,
            &attested_header,
            &sync_committee_signature.0.0,
            signature_slot,
        ).await;
    
        debug!("is_valid_sig = {is_valid_sig}");
        if !is_valid_sig {
            return;
        }
    }

    if update_is_newer {
        debug!("update is newer");
        if merkle::is_finality_proof_valid(
            &attested_header,
            &finalized_header,
            &finality_branch,
        ) {
            state.checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());
            state.finalized_header = finalized_header;
        } else {
            debug!("ConsensusError::InvalidFinalityProof.into()");
        }
    }

    if !update_has_finalized_next_committee {
        debug!("update doesn't have next committee");
        return;
    }

    // this check is only for the init-case when there is no next committee.
    if !update_is_newer && state.sync_committee_next.is_some() {
        debug!("update has next committee but store has it too");
        return;
    }

    if sync_committee_next.is_some() && sync_committee_next_branch.is_some() {
        let is_valid = merkle::is_next_committee_proof_valid(
            &attested_header,
            &sync_committee_next.as_ref().unwrap(),
            &sync_committee_next_branch.as_ref().unwrap(),
        );

        if !is_valid {
            debug!("ConsensusError::InvalidNextSyncCommitteeProof.into()");
            return;
        }
    }

    if !utils::check_public_keys(&sync_committee_next
        .expect("checked above")
        .pubkeys
        .0,
        sync_committee_next_pub_keys.as_ref().expect("checked above"),
    ) {
        debug!("Wrong public committee keys");
        return;
    }

    match state.sync_committee_next.take() {
        Some(stored_next_sync_committee) => {
            if update_finalized_period == store_period + 1 {
                debug!("sync committee updated");
                state.sync_committee_current = stored_next_sync_committee;
                state.sync_committee_next = sync_committee_next_pub_keys;
            } else {
                state.sync_committee_next = Some(stored_next_sync_committee);
            }
        }

        None => {
            state.sync_committee_next = sync_committee_next_pub_keys;
        }
    }
}

fn get_participating_keys(
    committee: &SyncCommitteeKeys,
    bitfield: &Bitvector<SYNC_COMMITTEE_SIZE>,
) -> Vec<G1> {
    bitfield.iter().zip(committee.0.iter())
        .filter_map(|(bit, pub_key)| {
            bit.then_some(pub_key.clone().0.0)
        })
        .collect::<Vec<_>>()
}
