use super::*;
use io::{
    ethereum_common::{beacon::BLSPubKey, SYNC_COMMITTEE_SIZE},
    SyncCommitteeKeys,
};

pub fn check_public_keys(
    compressed_keys: &[BLSPubKey; SYNC_COMMITTEE_SIZE],
    public_keys: &SyncCommitteeKeys,
) -> bool {
    let mut buffer = Vec::with_capacity(100);

    for (pub_key_compressed, pub_key) in compressed_keys.iter().zip(public_keys.0.iter()) {
        buffer.clear();

        if !matches!(
            <G1 as CanonicalSerialize>::serialize_compressed(&pub_key.0.0, &mut buffer),
            Ok(_) if pub_key_compressed.as_ref() == &buffer[..],
        ) {
            return false;
        }
    }

    true
}
