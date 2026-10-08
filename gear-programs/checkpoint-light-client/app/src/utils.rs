use ark_ec::CurveGroup;
use ark_serialize::CanonicalSerialize;
use checkpoint_light_client_io::{SyncCommitteeKeys, G1};
use ethereum_common::{
    base_types::{Bitvector, BytesFixed, FixedArray},
    beacon::{BLSPubKey, SyncCommittee},
    SYNC_COMMITTEE_SIZE,
};
use sails_rs::prelude::*;

pub fn construct_sync_committee(
    aggregate_pubkey: BLSPubKey,
    public_keys: &SyncCommitteeKeys,
) -> Option<SyncCommittee> {
    let mut pub_keys = Vec::with_capacity(SYNC_COMMITTEE_SIZE);
    for pub_key in public_keys.0.iter() {
        let pub_key = pub_key.0 .0.into_affine();
        // The authenticated committee root binds the unique compressed point.
        // Reject omitted-coordinate malleability before authenticating that root.
        if !pub_key.is_on_curve() {
            return None;
        }
        let mut buffer = BytesFixed(FixedArray([0u8; 48]));

        pub_key.serialize_compressed(buffer.0 .0.as_mut()).ok()?;

        pub_keys.push(buffer);
    }

    Some(SyncCommittee {
        pubkeys: FixedArray(pub_keys.try_into().ok()?),
        aggregate_pubkey,
    })
}

pub fn get_participating_keys(
    committee: &SyncCommitteeKeys,
    bitfield: &Bitvector<SYNC_COMMITTEE_SIZE>,
) -> Vec<G1> {
    bitfield
        .iter()
        .zip(committee.0.iter())
        .filter_map(|(bit, pub_key)| bit.then_some(pub_key.clone().0 .0))
        .collect()
}
