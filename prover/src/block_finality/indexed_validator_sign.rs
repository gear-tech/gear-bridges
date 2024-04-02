use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use plonky2_ed25519::gadgets::eddsa::make_verify_circuits as ed25519_circuit;
use plonky2_field::types::Field;

use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Ed25519PublicKeyTarget, Sha256Target, TargetSet},
        BuilderExt,
    },
    consts::GRANDPA_VOTE_LENGTH,
    prelude::*,
    ProofWithCircuitData,
};

use super::*;

impl_target_set! {
    pub struct IndexedValidatorSignTarget {
        pub validator_set_hash: Sha256Target,
        pub validator_idx: Target,
        pub message: GrandpaVoteTarget,
    }
}

pub struct IndexedValidatorSign {
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    pub index: usize,
    pub message: [u8; GRANDPA_VOTE_LENGTH],
    pub signature: [u8; consts::ED25519_SIGNATURE_SIZE],
}

impl IndexedValidatorSign {
    pub fn prove(
        &self,
        valiadtor_set_hash_proof: &ProofWithCircuitData<ValidatorSetHashTarget>,
    ) -> ProofWithCircuitData<IndexedValidatorSignTarget> {
        log::info!("    Proving indexed validator sign...");

        let sign_proof = SingleValidatorSign {
            public_key: self.public_key,
            signature: self.signature,
            message: self.message,
        }
        .prove();

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let validator_set_hash_target =
            builder.recursively_verify_constant_proof(valiadtor_set_hash_proof, &mut witness);

        let index_target = builder.add_virtual_target();
        witness.set_target(index_target, F::from_canonical_usize(self.index));

        let validator = validator_set_hash_target
            .validator_set
            .random_read(index_target, &mut builder);

        let sign_target = builder.recursively_verify_constant_proof(&sign_proof, &mut witness);

        validator.connect(&sign_target.public_key, &mut builder);

        IndexedValidatorSignTarget {
            validator_set_hash: validator_set_hash_target.hash,
            validator_idx: index_target,
            message: sign_target.message,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}

impl_target_set! {
    struct SingleValidatorSignTarget {
        message: GrandpaVoteTarget,
        public_key: Ed25519PublicKeyTarget,
    }
}

struct SingleValidatorSign {
    public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    signature: [u8; consts::ED25519_SIGNATURE_SIZE],
    message: [u8; GRANDPA_VOTE_LENGTH],
}

impl SingleValidatorSign {
    fn prove(&self) -> ProofWithCircuitData<SingleValidatorSignTarget> {
        log::info!("        Proving single validator sign...");

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        // This fn registers public inputs as:
        //  - message contents as `BoolTarget`s
        //  - public key as `BoolTarget`s
        let targets = ed25519_circuit(&mut builder, self.message.len());

        let mut pw = PartialWitness::new();

        let pk_bits = array_to_bits(&self.public_key).into_iter();
        for (target, value) in targets.pk.iter().zip(pk_bits) {
            pw.set_bool_target(*target, value);
        }

        let signature_bits = array_to_bits(&self.signature).into_iter();
        for (target, value) in targets.sig.iter().zip(signature_bits) {
            pw.set_bool_target(*target, value);
        }

        let msg_bits = array_to_bits(&self.message).into_iter();
        for (target, value) in targets.msg.iter().zip(msg_bits) {
            pw.set_bool_target(*target, value);
        }

        let proof = ProofWithCircuitData::from_builder(builder, pw);

        log::info!("        Proven single validator sign...");

        proof
    }
}
