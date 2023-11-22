use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    block_finality::BlockFinality,
    common::{ProofCompositionBuilder, ProofCompositionTargets},
    merkle_proof::MerkleProof,
    prelude::*,
    validator_set_hash::ValidatorSetHash,
    ProofWithCircuitData,
};

const PUBLIC_KEY_SIZE: usize = 32;
const PUBLIC_KEY_SIZE_IN_BITS: usize = PUBLIC_KEY_SIZE * 8;

const AUTHORITY_WEIGHT_SIZE: usize = 8;

const SHA256_DIGEST_SIZE: usize = 32;
const SHA256_DIGEST_SIZE_IN_BITS: usize = SHA256_DIGEST_SIZE * 8;

const BLAKE2_DIGEST_SIZE: usize = 32;
const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;

/// Public inputs:
/// - validator set hash
/// - next validator set hash
pub struct NextValidatorSet {
    pub current_epoch_block_finality: BlockFinality,
    pub next_validator_set_inclusion_proof: MerkleProof,
}

impl NextValidatorSet {
    pub fn prove(&self) -> ProofWithCircuitData {
        let next_validator_set_data = self.next_validator_set_inclusion_proof.leaf_data.clone();
        let next_validator_set_len =
            (next_validator_set_data.len() - 1) / (PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE);
        let mut next_validator_set = vec![];
        for validator_idx in 0..next_validator_set_len {
            next_validator_set.push(
                next_validator_set_data[1 + validator_idx
                    * (PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE)
                    ..1 + validator_idx * (PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE)
                        + PUBLIC_KEY_SIZE]
                    .try_into()
                    .unwrap(),
            );
        }

        let validator_set_hash_proof = ValidatorSetHash {
            validator_set: next_validator_set,
        }
        .prove();

        let non_hashed_next_validator_set_proof = NextValidatorSetNonHashed {
            current_epoch_block_finality: self.current_epoch_block_finality.clone(),
            next_validator_set_inclusion_proof: self.next_validator_set_inclusion_proof.clone(),
        }
        .prove();

        let composition_builder = ProofCompositionBuilder::new(
            validator_set_hash_proof,
            non_hashed_next_validator_set_proof,
        );

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let validator_set_hash_public_inputs = targets.first_proof_public_input_targets;
            let next_validator_set_public_inputs = targets.second_proof_public_input_targets;

            // Register current validator set hash as public input.
            for target in &next_validator_set_public_inputs[..SHA256_DIGEST_SIZE_IN_BITS] {
                builder.register_public_input(*target);
            }

            // Register next validator set hash as public input.
            for target in &validator_set_hash_public_inputs[..SHA256_DIGEST_SIZE_IN_BITS] {
                builder.register_public_input(*target);
            }

            // Assert that next validator set == one from hashing proof.
            let next_validator_set_0 =
                &validator_set_hash_public_inputs[SHA256_DIGEST_SIZE_IN_BITS..];
            let next_validator_set_1 =
                &next_validator_set_public_inputs[SHA256_DIGEST_SIZE_IN_BITS..];
            for (target_0, target_1) in next_validator_set_0.iter().zip(next_validator_set_1.iter())
            {
                builder.connect(*target_0, *target_1);
            }
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}

/// Public inputs:
/// - current validator set hash
/// - next validator set
struct NextValidatorSetNonHashed {
    current_epoch_block_finality: BlockFinality,
    next_validator_set_inclusion_proof: MerkleProof,
}

impl NextValidatorSetNonHashed {
    pub fn prove(&self) -> ProofWithCircuitData {
        // Public inputs:
        // - leaf data
        // - node #0 left data
        // - node #0 right data
        // - node #1 left data
        // - ...
        // - node #n right data
        // - root hash
        let merkle_tree_proof = self.next_validator_set_inclusion_proof.prove();

        // Public inputs:
        // - validator set hash
        // - message
        let block_finality_proof = self.current_epoch_block_finality.prove();

        let composition_builder =
            ProofCompositionBuilder::new(merkle_tree_proof, block_finality_proof);

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let merkle_proof_public_inputs = targets.first_proof_public_input_targets;
            let block_finality_public_inputs = targets.second_proof_public_input_targets;

            // Register current validator set hash as public input.
            for target in &block_finality_public_inputs[..SHA256_DIGEST_SIZE_IN_BITS] {
                builder.register_public_input(*target);
            }

            // Assume the next layout of leaf data:
            // - vector length          (1 byte)
            // - public key #1          (32 bytes)
            // - authority weight #1    (8 bytes)
            // - public key #2          (32 bytes)
            // ...
            // - authority weight #N    (8 bytes)

            // And the next layout for message:
            // - ???                    (1 byte)
            // - block hash             (32 bytes)
            // - block number           (4 bytes)
            // - round number           (8 bytes)
            // - authority set id       (8 bytes)

            // Register next validator set as public input.
            let validator_count = (self.next_validator_set_inclusion_proof.leaf_data.len() - 1)
                / (PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE);
            for validator_idx in 0..validator_count {
                let validator_public_key_start =
                    (1 + validator_idx * (PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE)) * 8;
                let validator_public_key_end = validator_public_key_start + PUBLIC_KEY_SIZE_IN_BITS;
                for target in &merkle_proof_public_inputs
                    [validator_public_key_start..validator_public_key_end]
                {
                    builder.register_public_input(*target);
                }
            }

            // Assert that merkle trie root hash == block hash from message.
            let message_block_hash = &block_finality_public_inputs[SHA256_DIGEST_SIZE_IN_BITS + 8
                ..SHA256_DIGEST_SIZE_IN_BITS + 8 + BLAKE2_DIGEST_SIZE_IN_BITS];
            let merkle_tree_root_hash = &merkle_proof_public_inputs
                [merkle_proof_public_inputs.len() - BLAKE2_DIGEST_SIZE_IN_BITS..];
            for (target_0, target_1) in message_block_hash.iter().zip(merkle_tree_root_hash.iter())
            {
                builder.connect(*target_0, *target_1);
            }
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}
