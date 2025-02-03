use ark_serialize::CanonicalDeserialize;
use checkpoint_light_client_io::{
    ArkScale, G1TypeInfo, G2TypeInfo, Keys as SyncCommitteeKeys, Update as SyncCommitteeUpdate, G1,
    G2,
};
use ethereum_common::{
    base_types::{BytesFixed, FixedArray},
    beacon::BLSPubKey,
    utils::{FinalityUpdate, Update},
    SYNC_COMMITTEE_SIZE,
};

pub fn sync_update_from_finality(
    signature: G2,
    finality_update: FinalityUpdate,
) -> SyncCommitteeUpdate {
    SyncCommitteeUpdate {
        signature_slot: finality_update.signature_slot,
        attested_header: finality_update.attested_header,
        finalized_header: finality_update.finalized_header,
        sync_committee_next_aggregate_pubkey: None,
        sync_committee_signature: G2TypeInfo(signature).into(),
        sync_committee_next_pub_keys: None,
        sync_committee_next_branch: None,
        finality_branch: finality_update
            .finality_branch
            .into_iter()
            .map(|BytesFixed(array)| array.0)
            .collect::<_>(),
    }
}

pub fn map_public_keys(
    compressed_public_keys: &FixedArray<BLSPubKey, SYNC_COMMITTEE_SIZE>,
) -> Box<SyncCommitteeKeys> {
    let keys = compressed_public_keys
        .0
        .iter()
        .map(|BytesFixed(pub_key_compressed)| {
            let pub_key = <G1 as CanonicalDeserialize>::deserialize_compressed_unchecked(
                &pub_key_compressed.0[..],
            )
            .expect("Public keys have the required size");

            let ark_scale: ArkScale<G1TypeInfo> = G1TypeInfo(pub_key).into();

            ark_scale
        })
        .collect::<Vec<_>>();

    Box::new(FixedArray(keys.try_into().expect(
        "The size of keys array is guaranteed on the type level",
    )))
}

pub fn sync_update_from_update(signature: G2, update: Update) -> SyncCommitteeUpdate {
    let next_sync_committee_keys = map_public_keys(&update.next_sync_committee.pubkeys);

    SyncCommitteeUpdate {
        signature_slot: update.signature_slot,
        attested_header: update.attested_header,
        finalized_header: update.finalized_header,
        sync_committee_next_aggregate_pubkey: Some(update.next_sync_committee.aggregate_pubkey),
        sync_committee_signature: G2TypeInfo(signature).into(),
        sync_committee_next_pub_keys: Some(next_sync_committee_keys),
        sync_committee_next_branch: Some(
            update
                .next_sync_committee_branch
                .into_iter()
                .map(|BytesFixed(array)| array.0)
                .collect::<_>(),
        ),
        finality_branch: update
            .finality_branch
            .into_iter()
            .map(|BytesFixed(array)| array.0)
            .collect::<_>(),
    }
}
