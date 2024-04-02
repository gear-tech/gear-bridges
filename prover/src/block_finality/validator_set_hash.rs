use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_field::types::Field;
use sp_core::blake2_256;

use crate::{
    common::{
        generic_blake2::GenericBlake2,
        targets::{Blake2Target, PaddedValidatorSetTarget},
        BuilderExt,
    },
    consts::ED25519_PUBLIC_KEY_SIZE,
    impl_target_set,
    prelude::*,
    ProofWithCircuitData,
};

use self::consts::BLAKE2_DIGEST_SIZE;

impl_target_set! {
    pub struct ValidatorSetHashTarget {
        pub hash: Blake2Target,
        pub validator_set: PaddedValidatorSetTarget,
        pub validator_set_length: Target
    }
}

pub struct ValidatorSetHash {
    pub validator_set: Vec<[u8; ED25519_PUBLIC_KEY_SIZE]>,
}

impl ValidatorSetHash {
    pub fn compute_hash(&self) -> [u8; BLAKE2_DIGEST_SIZE] {
        blake2_256(
            &self
                .validator_set
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
        )
    }

    pub fn prove(self) -> ProofWithCircuitData<ValidatorSetHashTarget> {
        log::info!("Proving correct hashing of validator set...");

        let validator_count = self.validator_set.len();

        let hasher_proof = GenericBlake2 {
            data: self.validator_set.into_iter().flatten().collect(),
        }
        .prove();

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());
        let mut pw = PartialWitness::new();

        let length_target = builder.add_virtual_target();
        pw.set_target(length_target, F::from_canonical_usize(validator_count));

        let hasher_pis = builder.recursively_verify_constant_proof(&hasher_proof, &mut pw);

        // let message_len_in_bits = VALIDATOR_COUNT * ED25519_PUBLIC_KEY_SIZE_IN_BITS;
        // let targets = sha256_circuit(&mut builder, message_len_in_bits);

        // for target in &targets.digest {
        //     builder.register_public_input(target.target);
        // }

        // // The message gets padded so we register only first `message_len_in_bits` bits.
        // for target in &targets.message[..message_len_in_bits] {
        //     builder.register_public_input(target.target);
        // }

        // let hash_bits = array_to_bits(&self.compute_hash());
        // for (target, value) in targets.digest.iter().zip(hash_bits) {
        //     pw.set_bool_target(*target, value);
        // }

        // let validator_set_bits = self.validator_set.iter().flat_map(|v| array_to_bits(v));
        // for (target, value) in targets.message.iter().zip(validator_set_bits) {
        //     pw.set_bool_target(*target, value);
        // }

        ProofWithCircuitData::from_builder(builder, pw)
    }
}
