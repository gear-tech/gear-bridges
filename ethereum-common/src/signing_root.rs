//! Module contains structs and functions helping calculate signing root.

use super::{Hash256, TreeHash};

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#signingdata).
#[derive(Default, Debug, tree_hash_derive::TreeHash)]
struct SigningData {
    object_root: [u8; 32],
    domain: [u8; 32],
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#forkdata).
#[derive(Default, Debug, tree_hash_derive::TreeHash)]
struct ForkData {
    current_version: [u8; 4],
    genesis_validator_root: [u8; 32],
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_signing_root).
pub fn compute(object_root: Hash256, domain: [u8; 32]) -> Hash256 {
    let data = SigningData {
        object_root: object_root.0,
        domain,
    };

    data.tree_hash_root()
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_domain).
pub fn compute_domain(
    domain_type: [u8; 4],
    fork_version: [u8; 4],
    genesis_validator_root: Hash256,
) -> [u8; 32] {
    let fork_data_root = compute_fork_data_root(fork_version, genesis_validator_root);
    let start = &domain_type[..];
    let end = &fork_data_root.as_ref()[..28];
    let d = [start, end].concat();

    d.to_vec().try_into().expect("Arrays length are correct")
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_fork_data_root).
pub fn compute_fork_data_root(
    current_version: [u8; 4],
    genesis_validator_root: Hash256,
) -> Hash256 {
    let fork_data = ForkData {
        current_version,
        genesis_validator_root: genesis_validator_root.0,
    };

    fork_data.tree_hash_root()
}
