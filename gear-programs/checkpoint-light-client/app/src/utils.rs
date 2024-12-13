use ethereum_common::{
    base_types::{BytesFixed, Bitvector, FixedArray},
    beacon::{BLSPubKey, SyncCommittee},
    SYNC_COMMITTEE_SIZE,
};
use crate::sync_committee::{self, G1};
use sails_rs::prelude::*;
use ark_serialize::CanonicalSerialize;

pub fn construct_sync_committee(
    aggregate_pubkey: BLSPubKey,
    public_keys: &sync_committee::Keys,
) -> Option<SyncCommittee> {
    let mut pub_keys = Vec::with_capacity(SYNC_COMMITTEE_SIZE);
    for pub_key in public_keys.0.iter() {
        let mut buffer = BytesFixed(FixedArray([0u8; 48]));

        if <G1 as CanonicalSerialize>::serialize_compressed(&pub_key.0 .0, buffer.0 .0.as_mut())
            .is_err()
        {
            return None;
        }

        pub_keys.push(buffer);
    }

    Some(SyncCommittee {
        pubkeys: FixedArray(pub_keys.try_into().ok()?),
        aggregate_pubkey,
    })
}

pub fn get_participating_keys(
    committee: &sync_committee::Keys,
    bitfield: &Bitvector<SYNC_COMMITTEE_SIZE>,
) -> Vec<G1> {
    bitfield
        .iter()
        .zip(committee.0.iter())
        .filter_map(|(bit, pub_key)| bit.then_some(pub_key.clone().0 .0))
        .collect()
}
