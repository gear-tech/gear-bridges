use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_sha256::circuit::make_circuits as sha256_circuit;
use sha2::{Digest, Sha256};

use crate::{
    common::{
        array_to_bits,
        targets::{Sha256Target, TargetSet, ValidatorSetTarget},
    },
    consts::VALIDATOR_COUNT,
    impl_target_set,
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct ValidatorSetHashTarget {
        pub hash: Sha256Target,
        pub validator_set: ValidatorSetTarget,
    }
}

pub struct ValidatorSetHash {
    pub validator_set: [[u8; consts::ED25519_PUBLIC_KEY_SIZE]; VALIDATOR_COUNT],
}

impl ValidatorSetHash {
    pub fn prove(&self) -> ProofWithCircuitData<ValidatorSetHashTarget> {
        log::info!("Proving correct hashing of validator set...");

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());

        let targets = sha256_circuit(
            &mut builder,
            (self.validator_set.len() * consts::ED25519_PUBLIC_KEY_SIZE_IN_BITS) as u64,
        );

        for target in &targets.digest {
            builder.register_public_input(target.target);
        }

        for target in &targets.message {
            builder.register_public_input(target.target);
        }

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

        ProofWithCircuitData::from_builder(builder, pw)
    }
}
