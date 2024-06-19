use super::*;
use io::{
    ethereum_common::{base_types::Bitvector, utils as eth_utils, SYNC_COMMITTEE_SIZE},
    SyncCommitteeKeys, SyncCommittee,
};
use gstd::debug;

enum Status<'a> {
    Actual {
        update_period_finalized: u64,
        attested_header: &'a BeaconBlockHeader,
        sync_committee_next: &'a SyncCommittee,
        sync_committee_next_pub_keys: Box<SyncCommitteeKeys>,
        sync_committee_next_branch: Vec<[u8; 32]>,
    },
    NotActual,
}

pub struct Update<'a>(Status<'a>);

impl<'a> Update<'a> {
    pub fn new(
        attested_header: &'a BeaconBlockHeader,
        update_slot_finalized: u64,
        sync_committee_next: Option<&'a SyncCommittee>,
        sync_committee_next_pub_keys: Option<Box<SyncCommitteeKeys>>,
        sync_committee_next_branch: Option<Vec<[u8; 32]>>,
    ) -> Self {
        let update_period_finalized = eth_utils::calculate_period(update_slot_finalized);
        match (
            eth_utils::calculate_period(attested_header.slot) == update_period_finalized,
            sync_committee_next,
            sync_committee_next_pub_keys,
            sync_committee_next_branch,
        ) {
            (
                true,
                Some(sync_committee_next),
                Some(sync_committee_next_pub_keys),
                Some(sync_committee_next_branch),
            ) => Self(Status::Actual {
                    update_period_finalized,
                    attested_header,
                    sync_committee_next,
                    sync_committee_next_pub_keys,
                    sync_committee_next_branch,
                }),

            _ => Self(Status::NotActual),
        }
    }

    pub fn actual(&self) -> bool {
        matches!(self.0, Status::Actual { .. })
    }

    pub fn apply(self, state: &mut State<COUNT>) {
        let Status::Actual {
            update_period_finalized,
            attested_header,
            sync_committee_next,
            sync_committee_next_pub_keys,
            sync_committee_next_branch,
        } = self.0 else {
            return;
        };

        let is_valid = merkle::is_next_committee_proof_valid(
            &attested_header,
            &sync_committee_next,
            &sync_committee_next_branch,
        );

        if !is_valid {
            debug!("ConsensusError::InvalidNextSyncCommitteeProof.into()");
            return;
        }

        if !utils::check_public_keys(
            &sync_committee_next.pubkeys.0,
            &sync_committee_next_pub_keys,
        ) {
            debug!("Wrong public committee keys");
            return;
        }

        let store_period = eth_utils::calculate_period(state.finalized_header.slot);
        match state.sync_committee_next.take() {
            Some(stored_next_sync_committee) => {
                if update_period_finalized == store_period + 1 {
                    debug!("sync committee updated");
                    state.sync_committee_current = stored_next_sync_committee;
                    state.sync_committee_next = Some(sync_committee_next_pub_keys);
                } else {
                    state.sync_committee_next = Some(stored_next_sync_committee);
                }
            }
    
            None => {
                state.sync_committee_next = Some(sync_committee_next_pub_keys);
            }
        }
    }
}
