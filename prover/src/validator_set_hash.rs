use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_sha256::circuit::sha256_circuit;
use sha2::{Digest, Sha256};

use crate::{common::array_to_bits, prelude::*, ProofWithCircuitData};

const PUBLIC_KEY_SIZE: usize = 32;
const PUBLIC_KEY_SIZE_IN_BITS: usize = PUBLIC_KEY_SIZE * 8;

/// Public inputs:
/// - hash
/// - validator set
pub struct ValidatorSetHash {
    pub validator_set: Vec<[u8; PUBLIC_KEY_SIZE]>,
}

impl ValidatorSetHash {
    pub fn prove(&self) -> ProofWithCircuitData {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());

        let targets = sha256_circuit(
            &mut builder,
            self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS,
        );

        for target in &targets.digest {
            builder.register_public_input(target.target);
        }

        for target in &targets.message {
            builder.register_public_input(target.target);
        }

        let circuit_data = builder.build::<C>();

        let mut pw = PartialWitness::new();

        let mut hasher = Sha256::new();
        hasher.update(
            &self
                .validator_set
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
        );
        let hash = hasher.finalize();
        let hash_bits = array_to_bits(&hash);
        for (target, value) in targets.digest.iter().zip(hash_bits) {
            pw.set_bool_target(*target, value);
        }

        let validator_set_bits = self.validator_set.iter().flat_map(|v| array_to_bits(v));
        for (target, value) in targets.message.iter().zip(validator_set_bits) {
            pw.set_bool_target(*target, value);
        }

        let proof = circuit_data.prove(pw).unwrap();

        ProofWithCircuitData {
            proof,
            circuit_data,
        }
    }
}
