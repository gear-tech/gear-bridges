use super::*;
use io::ethereum_common::{
    base_types::BytesFixed,
    beacon::{BLSPubKey, SyncCommittee},
};

pub fn construct_sync_committee(
    aggregate_pubkey: BLSPubKey,
    public_keys: &SyncCommitteeKeys,
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
    committee: &SyncCommitteeKeys,
    bitfield: &Bitvector<SYNC_COMMITTEE_SIZE>,
) -> Vec<G1> {
    bitfield
        .iter()
        .zip(committee.0.iter())
        .filter_map(|(bit, pub_key)| bit.then_some(pub_key.clone().0 .0))
        .collect::<Vec<_>>()
}

pub fn construct_state_reply(
    request: meta::StateRequest,
    state: &State<STORED_CHECKPOINTS_COUNT>,
) -> meta::State {
    use meta::Order;

    let count = Saturating(request.count as usize);
    let checkpoints_all = state.checkpoints.checkpoints();
    let (start, end) = match request.order {
        Order::Direct => {
            let start = Saturating(request.index_start as usize);
            let Saturating(end) = start + count;

            (start.0, cmp::min(end, checkpoints_all.len()))
        }

        Order::Reverse => {
            let len = Saturating(checkpoints_all.len());
            let index_last = Saturating(request.index_start as usize);
            let end = len - index_last;
            let Saturating(start) = end - count;

            (start, end.0)
        }
    };

    let checkpoints = checkpoints_all
        .get(start..end)
        .map(|slice| slice.to_vec())
        .unwrap_or(vec![]);

    let replay_back = state
        .replay_back
        .as_ref()
        .map(|replay_back| meta::ReplayBack {
            finalized_header: replay_back.finalized_header.slot,
            last_header: replay_back.last_header.slot,
        });

    meta::State {
        checkpoints,
        replay_back,
    }
}
