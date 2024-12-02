//! Circuit that's used to prove that majority of validators have signed GRANDPA message.

use itertools::Itertools;
use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, CommonCircuitData},
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};
use plonky2_field::types::Field;
use rayon::{
    iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator},
    ThreadPoolBuilder,
};
use std::{iter, sync::mpsc::channel};

mod indexed_validator_sign;
mod single_validator_sign;

use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
        targets::{
            impl_parsable_target_set, impl_target_set, Blake2Target, ParsableTargetSet, TargetSet,
            VerifierDataTarget,
        },
        BuilderExt, ProofWithCircuitData,
    },
    prelude::{
        consts::{BLAKE2_DIGEST_SIZE, GRANDPA_VOTE_LENGTH},
        *,
    },
};

use self::indexed_validator_sign::IndexedValidatorSignTarget;

use indexed_validator_sign::IndexedValidatorSign;

use super::{validator_set_hash::ValidatorSetHash, GrandpaMessageTarget, ProcessedPreCommit};

const VALIDATOR_SIGN_PROVER_THREAD_MAX_STACK_SIZE: usize = 65_536 * 64;

impl_target_set! {
    /// Public inputs for `ValidatorSignsChain`.
    pub struct ValidatorSignsChainTarget {
        /// Blake2 hash of concatenated validator public keys.
        pub validator_set_hash: Blake2Target,
        /// GRANDPA message.
        pub message: GrandpaMessageTarget,
    }
}

pub struct ValidatorSignsChain {
    /// `ValidatorSetHash` proof builder.
    pub validator_set_hash: ValidatorSetHash,
    /// All the pre-commits that're planned to process(that is, prove that they're all signed message).
    pub pre_commits: Vec<ProcessedPreCommit>,
    /// GRANDPA message.
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl ValidatorSignsChain {
    pub fn prove(self) -> ProofWithCircuitData<ValidatorSignsChainTarget> {
        log::debug!("Proving validator signs chain...");

        let validator_set_hash = self.validator_set_hash.compute_hash();

        let validator_set_hash_proof = self.validator_set_hash.prove();

        let mut pre_commits = self.pre_commits.clone();
        pre_commits.sort_by(|a, b| a.validator_idx.cmp(&b.validator_idx));

        let (sender, receiver) = channel();

        let thread_pool = ThreadPoolBuilder::new()
            .stack_size(VALIDATOR_SIGN_PROVER_THREAD_MAX_STACK_SIZE)
            .build()
            .expect("Failed to create ThreadPool");

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
                    .prove(&validator_set_hash_proof);

                    sender
                        .send((id, proof))
                        .expect("Failed to send proof over channel");
                });
            },
        );

        let mut inner_proofs = receiver
            .iter()
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, proof)| proof)
            .collect::<Vec<_>>();

        let initial_data = SignCompositionInitialData {
            validator_set_hash,
            message: self.message,
        };
        let mut composed_proof =
            SignComposition::build(&inner_proofs.remove(0)).prove_initial(initial_data);

        for inner in inner_proofs {
            composed_proof = SignComposition::build(&inner).prove_recursive(composed_proof.proof());
        }

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let composed_proof_pis =
            builder.recursively_verify_constant_proof(&composed_proof, &mut witness);

        // Assert that sign_count > 2/3 * validator_count
        // 3 * sign_count - 2 * validator_count - 1 >= 0
        {
            let triple_sign_count =
                builder.mul_const(F::from_canonical_usize(3), composed_proof_pis.sign_count);
            let double_validator_count =
                builder.mul_const(F::TWO, composed_proof_pis.validator_count);
            let lhs = builder.sub(triple_sign_count, double_validator_count);
            let lhs = builder.add_const(lhs, F::NEG_ONE);
            builder.range_check(lhs, 32);
        }

        ValidatorSignsChainTarget {
            validator_set_hash: composed_proof_pis.validator_set_hash,
            message: composed_proof_pis.message,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Proven validator signs chain");

        result
    }
}

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

// TODO: Put nested target here.
impl_target_set! {
    struct SignCompositionTarget {
        validator_set_hash: Blake2Target,
        validator_count: Target,

        message: GrandpaMessageTarget,

        latest_validator_idx: Target,
        sign_count: Target,

        verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

impl_parsable_target_set! {
    struct SignCompositionTargetWithoutCircuitData {
        validator_set_hash: Blake2Target,
        validator_set_count: Target,

        message: GrandpaMessageTarget,

        latest_validator_idx: Target,
        sign_count: Target,
    }
}

struct SignCompositionInitialData {
    validator_set_hash: [u8; BLAKE2_DIGEST_SIZE],
    message: [u8; GRANDPA_VOTE_LENGTH],
}

/// Inner cyclic recursion proof.
struct SignComposition {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,

    witness: PartialWitness<F>,
}

impl SignComposition {
    fn prove_initial(
        mut self,
        initial_data: SignCompositionInitialData,
    ) -> ProofWithCircuitData<SignCompositionTarget> {
        log::debug!("    Proving sign composition recursion layer(initial)...");

        let validator_set_hash = array_to_bits(&initial_data.validator_set_hash);
        let message = array_to_bits(&initial_data.message);

        let public_inputs = validator_set_hash
            .into_iter()
            .map(|bit| bit as usize)
            .chain(iter::once(0))
            .chain(message.into_iter().map(|bit| bit as usize))
            .chain(iter::once(0))
            .chain(iter::once(0))
            .map(F::from_canonical_usize);

        // Length check.
        SignCompositionTargetWithoutCircuitData::parse_public_inputs_exact(
            &mut public_inputs.clone(),
        );

        let public_inputs = public_inputs.enumerate().collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

        log::debug!("    Proven sign composition recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<SignCompositionTarget> {
        log::debug!("    Proving sign composition recursion layer...");
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

        log::debug!("    Proven sign composition recursion layer");

        result
    }

    fn build(inner_proof: &ProofWithCircuitData<IndexedValidatorSignTarget>) -> SignComposition {
        log::debug!("    Building sign composition recursion layer...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut pw = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(inner_proof, &mut pw);

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            SignCompositionTargetWithoutCircuitData::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(
            CircuitConfig::standard_recursion_config(),
            builder.num_public_inputs(),
            1 << 13,
        );

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = SignCompositionTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = SignCompositionTargetWithoutCircuitData {
            validator_set_hash: inner_cyclic_proof_pis.validator_set_hash,
            validator_set_count: inner_cyclic_proof_pis.validator_count,

            message: inner_cyclic_proof_pis.message,

            latest_validator_idx: inner_cyclic_proof_pis.latest_validator_idx,
            sign_count: inner_cyclic_proof_pis.sign_count,
        };

        let neg_one = builder.neg_one();
        inner_cyclic_proof_pis.latest_validator_idx = builder.select(
            condition,
            inner_cyclic_proof_pis.latest_validator_idx,
            neg_one,
        );

        let zero = builder.zero();
        inner_cyclic_proof_pis.sign_count =
            builder.select(condition, inner_cyclic_proof_pis.sign_count, zero);

        inner_cyclic_proof_pis.validator_set_count = builder.select(
            condition,
            inner_cyclic_proof_pis.validator_set_count,
            inner_proof_pis.validator_count,
        );

        inner_proof_pis
            .message
            .connect(&inner_cyclic_proof_pis.message, &mut builder);
        inner_proof_pis
            .validator_set_hash
            .connect(&inner_cyclic_proof_pis.validator_set_hash, &mut builder);
        inner_proof_pis
            .validator_count
            .connect(&inner_cyclic_proof_pis.validator_set_count, &mut builder);

        let validator_idx_diff = builder.sub(
            inner_proof_pis.validator_idx,
            inner_cyclic_proof_pis.latest_validator_idx,
        );
        let to_compare_with_zero = builder.add_const(validator_idx_diff, F::NEG_ONE);
        builder.range_check(to_compare_with_zero, 32);

        let sign_count = builder.add_const(inner_cyclic_proof_pis.sign_count, F::ONE);

        let final_pis = SignCompositionTargetWithoutCircuitData {
            validator_set_hash: inner_cyclic_proof_pis.validator_set_hash,
            validator_set_count: inner_cyclic_proof_pis.validator_set_count,
            message: inner_cyclic_proof_pis.message,
            latest_validator_idx: inner_proof_pis.validator_idx,
            sign_count,
        };

        future_inner_cyclic_proof_pis.connect(&final_pis, &mut builder);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                condition,
                &inner_cyclic_proof_with_pis,
                &common_data,
            )
            .expect("Failed to build circuit");

        let cyclic_circuit_data = builder.build::<C>();

        pw.set_verifier_data_target(&verifier_data_target, &cyclic_circuit_data.verifier_only);

        log::debug!("    Built sign composition recursion layer");

        SignComposition {
            cyclic_circuit_data,
            common_data,
            condition,
            inner_cyclic_proof_with_pis,
            witness: pw,
        }
    }
}
