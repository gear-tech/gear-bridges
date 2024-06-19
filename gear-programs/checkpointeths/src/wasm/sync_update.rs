use super::*;
use gstd::debug;
use io::{
    ethereum_common::{utils as eth_utils, SYNC_COMMITTEE_SIZE},
    SyncCommitteeKeys,
};
use committee::{Update as CommitteeUpdate, Error as CommitteeError};

pub async fn handle(state: &mut State<COUNT>, sync_update: SyncUpdate) {
    let (finalized_header_update, committee_update) = match verify(
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

    if let Some(finalized_header) = finalized_header_update {
        state.checkpoints.push(finalized_header.slot, finalized_header.tree_hash_root());
        state.finalized_header = finalized_header;
    }

    if let Some(sync_committee_next) = committee_update {
        state.sync_committee_current = core::mem::replace(&mut state.sync_committee_next, sync_committee_next);
    }
}

#[derive(Debug, Clone)]
pub enum Error {
    InvalidTimestamp,
    InvalidPeriod,
    LowVoteCount,
    NotActual,
    InvalidSignature,
    InvalidFinalityProof,
    InvalidNextSyncCommitteeProof,
    InvalidPublicKeys,
}

impl From<CommitteeError> for Error {
    fn from(value: CommitteeError) -> Self {
        match value {
            CommitteeError::InvalidNextSyncCommitteeProof => Error::InvalidNextSyncCommitteeProof,
            CommitteeError::InvalidPublicKeys => Error::InvalidPublicKeys,
        }
    }
}

pub async fn verify(
    genesis: &Genesis,
    stored_finalized_header: &BeaconBlockHeader,
    stored_sync_committee_current: &SyncCommitteeKeys,
    stored_sync_committee_next: &SyncCommitteeKeys,
    sync_update: SyncUpdate,
) -> Result<(Option<BeaconBlockHeader>, Option<Box<SyncCommitteeKeys>>), Error> {
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
    if !(signature_slot > attested_header.slot && attested_header.slot >= update_slot_finalized) {
        return Err(Error::InvalidTimestamp);
    }

    let store_period = eth_utils::calculate_period(stored_finalized_header.slot);
    let update_sig_period = eth_utils::calculate_period(signature_slot);
    let sync_committee = if update_sig_period == store_period + 1 {
        stored_sync_committee_next
    } else if update_sig_period == store_period {
        stored_sync_committee_current
    } else {
        return Err(Error::InvalidPeriod);
    };

    let pub_keys = utils::get_participating_keys(&sync_committee, &sync_aggregate.sync_committee_bits);
    let committee_count = pub_keys.len();

    // committee_count < 512 * 2 / 3
    if committee_count * 3 < SYNC_COMMITTEE_SIZE * 2 {
        return Err(Error::LowVoteCount);
    }

    let committee_update = CommitteeUpdate::new(
        &attested_header,
        update_slot_finalized,
        sync_committee_next.as_deref(),
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
    );

    if !crypto::verify_sync_committee_signature(
        genesis,
        pub_keys,
        &attested_header,
        &sync_committee_signature.0 .0,
        signature_slot,
    )
    .await {
        return Err(Error::InvalidSignature);
    }

    let mut finalized_header_update = None;
    if update_slot_finalized > stored_finalized_header.slot {
        if merkle::is_finality_proof_valid(&attested_header, &finalized_header, &finality_branch) {
            finalized_header_update = Some(finalized_header);
        } else {
            return Err(Error::InvalidFinalityProof);
        }
    }

    let committee_update = committee_update.verify(store_period)?;
    if finalized_header_update.is_none() && committee_update.is_none() {
        return Err(Error::NotActual);
    }

    Ok((finalized_header_update, committee_update))
}
