use super::{TreeHash, beacon::{BlockHeader as BeaconBlockHeader, SyncCommittee}};
use ring::digest::{Context as RingContext, SHA256 as RingSHA256};

pub const MERKLE_PROOF_DEPTH_CURRENT_SYNC_COMMITTEE: u32 = 5;
pub const MERKLE_PROOF_INDEX_CURRENT_SYNC_COMMITTEE: u32 = 22;
pub const MERKLE_PROOF_DEPTH_NEXT_SYNC_COMMITTEE: u32 = 5;
pub const MERKLE_PROOF_INDEX_NEXT_SYNC_COMMITTEE: u32 = 23;
pub const MERKLE_PROOF_DEPTH_FINALITY: u32 = 6;
pub const MERKLE_PROOF_INDEX_FINALITY: u32 = 41;

pub fn is_valid_merkle_branch(
    leaf: [u8; 32],
    branch: &[[u8; 32]],
    depth: u32,
    index: u32,
    root: &[u8; 32],
) -> bool {
    let mut value = leaf;

    let mut hasher = RingContext::new(&RingSHA256);
    let mut iter = branch.iter();
    for i in 0..depth {
        let Some(next_node) = iter.next() else {
            return false;
        };

        let (node_first, node_second) = match (index / 2u32.pow(i)) % 2 {
            0 => (value.as_ref(), next_node.as_ref()),
            _ => (next_node.as_ref(), value.as_ref()),
        };

        hasher.update(node_first);
        hasher.update(node_second);

        value.copy_from_slice(hasher.finish().as_ref());
        hasher = RingContext::new(&RingSHA256);
    }

    value == *root
}

pub fn is_current_committee_proof_valid(
    attested_header: &BeaconBlockHeader,
    current_committee: &SyncCommittee,
    current_committee_branch: &[[u8; 32]],
) -> bool {
    let leaf_hash = current_committee.tree_hash_root();
    let state_root = attested_header.state_root;

    is_valid_merkle_branch(leaf_hash.0, current_committee_branch, MERKLE_PROOF_DEPTH_CURRENT_SYNC_COMMITTEE, MERKLE_PROOF_INDEX_CURRENT_SYNC_COMMITTEE, &state_root.0)
}

pub fn is_finality_proof_valid(
    attested_header: &BeaconBlockHeader,
    finality_header: &BeaconBlockHeader,
    finality_branch: &[[u8; 32]],
) -> bool {
    let leaf_hash = finality_header.tree_hash_root();
    let state_root = attested_header.state_root;

    is_valid_merkle_branch(leaf_hash.0, finality_branch, MERKLE_PROOF_DEPTH_FINALITY, MERKLE_PROOF_INDEX_FINALITY, &state_root.0)
}

pub fn is_next_committee_proof_valid(
    attested_header: &BeaconBlockHeader,
    next_committee: &SyncCommittee,
    next_committee_branch: &[[u8; 32]],
) -> bool {
    let leaf_hash = next_committee.tree_hash_root();
    let state_root = attested_header.state_root;

    is_valid_merkle_branch(leaf_hash.0, next_committee_branch, MERKLE_PROOF_DEPTH_NEXT_SYNC_COMMITTEE, MERKLE_PROOF_INDEX_NEXT_SYNC_COMMITTEE, &state_root.0)
}
