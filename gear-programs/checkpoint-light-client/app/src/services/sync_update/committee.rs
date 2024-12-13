use super::*;

enum Status<'a> {
    Actual {
        update_period_finalized: u64,
        attested_header: &'a BeaconBlockHeader,
        sync_committee_next_aggregate_pubkey: BLSPubKey,
        sync_committee_next_pub_keys: Box<SyncCommitteeKeys>,
        sync_committee_next_branch: Vec<[u8; 32]>,
    },
    NotActual,
}

#[derive(Debug, Clone)]
pub enum Error {
    InvalidNextSyncCommitteeProof,
    InvalidPublicKeys,
}

pub struct Update<'a>(Status<'a>);

impl<'a> Update<'a> {
    pub fn new(
        attested_header: &'a BeaconBlockHeader,
        update_slot_finalized: u64,
        sync_committee_next_aggregate_pubkey: Option<BLSPubKey>,
        sync_committee_next_pub_keys: Option<Box<SyncCommitteeKeys>>,
        sync_committee_next_branch: Option<Vec<[u8; 32]>>,
    ) -> Self {
        let update_period_finalized = eth_utils::calculate_period(update_slot_finalized);
        match (
            eth_utils::calculate_period(attested_header.slot) == update_period_finalized,
            sync_committee_next_aggregate_pubkey,
            sync_committee_next_pub_keys,
            sync_committee_next_branch,
        ) {
            (
                true,
                Some(sync_committee_next_aggregate_pubkey),
                Some(sync_committee_next_pub_keys),
                Some(sync_committee_next_branch),
            ) => Self(Status::Actual {
                update_period_finalized,
                attested_header,
                sync_committee_next_aggregate_pubkey,
                sync_committee_next_pub_keys,
                sync_committee_next_branch,
            }),

            _ => Self(Status::NotActual),
        }
    }

    pub fn verify(self, store_period: u64) -> Result<Option<Box<SyncCommitteeKeys>>, Error> {
        let Status::Actual {
            update_period_finalized,
            attested_header,
            sync_committee_next_aggregate_pubkey,
            sync_committee_next_pub_keys,
            sync_committee_next_branch,
        } = self.0
        else {
            return Ok(None);
        };

        let sync_committee_next = utils::construct_sync_committee(
            sync_committee_next_aggregate_pubkey,
            &sync_committee_next_pub_keys,
        )
        .ok_or(Error::InvalidPublicKeys)?;

        if !merkle::is_next_committee_proof_valid(
            &attested_header,
            &sync_committee_next,
            &sync_committee_next_branch,
        ) {
            return Err(Error::InvalidNextSyncCommitteeProof);
        }

        if update_period_finalized == store_period + 1 {
            return Ok(Some(sync_committee_next_pub_keys));
        }

        Ok(None)
    }
}
