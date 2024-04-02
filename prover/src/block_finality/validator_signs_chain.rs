use itertools::Itertools;
use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use plonky2_ed25519::gadgets::eddsa::make_verify_circuits as ed25519_circuit;
use plonky2_field::types::Field;
use rayon::{
    iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator},
    ThreadPoolBuilder,
};
use std::sync::mpsc::channel;

use crate::{
    common::{
        array_to_bits,
        targets::{
            impl_target_set, BitArrayTarget, Blake2Target, Ed25519PublicKeyTarget, Sha256Target,
            TargetSet,
        },
        BuilderExt,
    },
    consts::{GRANDPA_VOTE_LENGTH, PROCESSED_VALIDATOR_COUNT, VALIDATOR_COUNT},
    prelude::*,
    ProofWithCircuitData,
};

use super::*;

const VALIDATOR_SIGN_PROVER_THREAD_MAX_STACK_SIZE: usize = 65_536 * 64;

impl_target_set! {
    pub struct ValidatorSignsChainTarget {
        pub validator_set_hash: Sha256Target,
        pub validator_idx: Target,
        pub message: GrandpaVoteTarget,
    }
}

pub struct ValidatorSignsChain {
    pub validator_set_hash_proof: ProofWithCircuitData<ValidatorSetHashTarget>,
    pub pre_commits: Vec<ProcessedPreCommit>,
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl ValidatorSignsChain {
    pub fn prove(&self) -> ProofWithCircuitData<ValidatorSignsChainTarget> {
        log::info!("Proving validator signs chain...");

        let mut pre_commits = self.pre_commits.clone();
        pre_commits.sort_by(|a, b| a.validator_idx.cmp(&b.validator_idx));

        let (sender, receiver) = channel();

        let thread_pool = ThreadPoolBuilder::new()
            .stack_size(VALIDATOR_SIGN_PROVER_THREAD_MAX_STACK_SIZE)
            .build()
            .unwrap();

        pre_commits.into_par_iter().enumerate().for_each_with(
            sender,
            |sender, (id, pre_commit)| {
                thread_pool.scope(|_| {
                    let proof = IndexedValidatorSign {
                        public_key: pre_commit.public_key,
                        index: pre_commit.validator_idx,
                        signature: pre_commit.signature,
                        message: self.message,
                    }
                    .prove(&self.validator_set_hash_proof);

                    sender.send((id, proof)).unwrap();
                });
            },
        );

        receiver
            .iter()
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, proof)| proof)
            .reduce(|acc, x| ComposedValidatorSigns {}.prove(acc, x))
            .unwrap()
    }
}

struct ComposedValidatorSigns {}

impl ComposedValidatorSigns {
    fn prove(
        &self,
        previous_composed_proof: ProofWithCircuitData<ValidatorSignsChainTarget>,
        indexed_sign_proof: ProofWithCircuitData<ValidatorSignsChainTarget>,
    ) -> ProofWithCircuitData<ValidatorSignsChainTarget> {
        log::info!("    Proving validator signs composition...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let previous_proof_target =
            builder.recursively_verify_constant_proof(&previous_composed_proof, &mut witness);
        let indexed_sign_target =
            builder.recursively_verify_constant_proof(&indexed_sign_proof, &mut witness);

        previous_proof_target
            .message
            .connect(&indexed_sign_target.message, &mut builder);

        previous_proof_target
            .validator_set_hash
            .connect(&indexed_sign_target.validator_set_hash, &mut builder);

        let new_index_sub_latest = builder.sub(
            indexed_sign_target.validator_idx,
            previous_proof_target.validator_idx,
        );
        let one = builder.one();
        let to_compare_with_0 = builder.sub(new_index_sub_latest, one);
        // Assert that `to_compare_with_0` >= 0.
        // This works because new_index_sub_latest << 2^32.
        builder.range_check(to_compare_with_0, 32);

        ValidatorSignsChainTarget {
            validator_set_hash: indexed_sign_target.validator_set_hash,
            validator_idx: indexed_sign_target.validator_idx,
            message: indexed_sign_target.message,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}

struct IndexedValidatorSign {
    public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    index: usize,
    message: [u8; GRANDPA_VOTE_LENGTH],
    signature: [u8; consts::ED25519_SIGNATURE_SIZE],
}

impl IndexedValidatorSign {
    fn prove(
        &self,
        valiadtor_set_hash_proof: &ProofWithCircuitData<ValidatorSetHashTarget>,
    ) -> ProofWithCircuitData<ValidatorSignsChainTarget> {
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

        ValidatorSignsChainTarget {
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
