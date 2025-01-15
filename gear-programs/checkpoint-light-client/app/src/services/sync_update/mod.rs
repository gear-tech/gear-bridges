mod committee;

use crate::{
    crypto,
    sync_committee::{Keys as SyncCommitteeKeys, Error as SyncCommitteeUpdateError, Update as SyncCommitteeUpdate},
    utils,
    common::Slot,
};
use ethereum_common::{beacon::{BLSPubKey, BlockHeader as BeaconBlockHeader, SyncAggregate}, merkle, network::Network, utils as eth_utils, SYNC_COMMITTEE_SIZE};
use sails_rs::prelude::*;

pub async fn verify(
    network: &Network,
    stored_finalized_slot: Slot,
    stored_sync_committee_current: &SyncCommitteeKeys,
    stored_sync_committee_next: &SyncCommitteeKeys,
    sync_update: SyncCommitteeUpdate,
    sync_aggregate: SyncAggregate,
) -> Result<(Option<BeaconBlockHeader>, Option<Box<SyncCommitteeKeys>>), SyncCommitteeUpdateError> {
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
        utils::get_participating_keys(&sync_committee, &sync_aggregate.sync_committee_bits);
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
        if merkle::is_finality_proof_valid(&attested_header, &finalized_header, &finality_branch) {
            finalized_header_update = Some(finalized_header);
        } else {
            return Err(SyncCommitteeUpdateError::InvalidFinalityProof);
        }
    }

    let committee_update = match committee_update.verify(store_period) {
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
