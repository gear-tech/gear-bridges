use super::*;
use gstd::debug;
use io::{
    ethereum_common::{utils as eth_utils, SYNC_COMMITTEE_SIZE},
    SyncCommitteeKeys,
};
use committee::Update as CommitteeUpdate;

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

    let update_slot_finalized = finalized_header.slot;
    let valid_time =
        signature_slot > attested_header.slot && attested_header.slot >= update_slot_finalized;
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

    let pub_keys = utils::get_participating_keys(&sync_committee, &sync_aggregate.sync_committee_bits);
    let committee_count = pub_keys.len();

    // committee_count < 512 * 2 / 3
    if committee_count * 3 < SYNC_COMMITTEE_SIZE * 2 {
        debug!("skipping block with low vote count");
        return;
    }

    let committee_update = CommitteeUpdate::new(
        &attested_header,
        update_slot_finalized,
        sync_committee_next.as_ref(),
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
    );

    let update_is_newer = update_slot_finalized > state.finalized_header.slot;
    if !(update_is_newer || committee_update.actual()) {
        debug!("Update is neither newer nor containing next sync committee update");
        return;
    }

    let is_valid_sig = crypto::verify_sync_committee_signature(
        &state.genesis,
        pub_keys,
        &attested_header,
        &sync_committee_signature.0 .0,
        signature_slot,
    )
    .await;

    debug!("is_valid_sig = {is_valid_sig}");
    if !is_valid_sig {
        return;
    }

    if update_is_newer {
        debug!("update is newer");
        if merkle::is_finality_proof_valid(&attested_header, &finalized_header, &finality_branch) {
            state
                .checkpoints
                .push(finalized_header.slot, finalized_header.tree_hash_root());
            state.finalized_header = finalized_header;
        } else {
            debug!("ConsensusError::InvalidFinalityProof.into()");
        }
    }

    committee_update.apply(state);
}
