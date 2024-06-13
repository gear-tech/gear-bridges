use super::*;
use io::{SyncCommittee, tree_hash::TreeHash, BeaconBlockHeader};
use ring::digest::{Context as RingContext, SHA256 as RingSHA256};

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

pub fn is_proof_valid(
    state_root: [u8; 32],
    leaf_hash: [u8; 32],
    branch: &[[u8; 32]],
    depth: u32,
    index: u32,
) -> bool {
    is_valid_merkle_branch(leaf_hash, branch, depth, index, &state_root)
}

pub fn is_current_committee_proof_valid(
    attested_header: &BeaconBlockHeader,
    current_committee: &SyncCommittee,
    current_committee_branch: &[[u8; 32]],
) -> bool {
    let leaf_hash = current_committee.tree_hash_root();

    is_proof_valid(
        attested_header.state_root.0,
        leaf_hash.0,
        current_committee_branch,
        5,
        22,
    )
}
