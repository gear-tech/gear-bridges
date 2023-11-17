use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    block_finality::BlockFinality, common::ProofCompositionTargets, merkle_proof::MerkleProof,
    prelude::*, ProofWithCircuitData,
};

const SHA256_DIGEST_SIZE: usize = 32;
const SHA256_DIGEST_SIZE_IN_BITS: usize = SHA256_DIGEST_SIZE * 8;

const BLAKE2_DIGEST_SIZE: usize = 32;
const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;

/// Public inputs:
/// - validator set hash
/// - message contents
pub struct MessageSent {
    pub block_finality: BlockFinality,
    pub inclusion_proof: MerkleProof,
}

impl MessageSent {
    pub fn prove(&self) -> ProofWithCircuitData {
        let inclusion_proof = self.inclusion_proof.prove();
        let finality_proof = self.block_finality.prove();

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let inclusion_proof_public_inputs = targets.first_proof_public_input_targets;
            let finality_proof_public_inputs = targets.second_proof_public_input_targets;

            // Register validator set hash as public input.
            for target in &finality_proof_public_inputs[..SHA256_DIGEST_SIZE_IN_BITS] {
                builder.register_public_input(*target);
            }

            // Register message contents as pblic input.
            let message_len_in_bits = self.inclusion_proof.leaf_data.len() * 8;
            for target in &inclusion_proof_public_inputs[..message_len_in_bits] {
                builder.register_public_input(*target);
            }

            // Assert that merkle trie root hash == block hash from message.
            let message_block_hash = &finality_proof_public_inputs[SHA256_DIGEST_SIZE_IN_BITS + 8
                ..SHA256_DIGEST_SIZE_IN_BITS + 8 + BLAKE2_DIGEST_SIZE_IN_BITS];
            let merkle_tree_root_hash = &inclusion_proof_public_inputs
                [inclusion_proof_public_inputs.len() - BLAKE2_DIGEST_SIZE_IN_BITS..];
            for (target_0, target_1) in message_block_hash.iter().zip(merkle_tree_root_hash.iter())
            {
                builder.connect(*target_0, *target_1);
            }
        };

        let proof = ProofWithCircuitData::compose(&inclusion_proof, &finality_proof, targets_op);

        proof
    }
}
