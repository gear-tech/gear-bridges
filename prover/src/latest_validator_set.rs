use itertools::Itertools;
use plonky2::{
    field::types::Field,
    hash::hash_types::{HashOut, HashOutTarget},
    iop::{target::BoolTarget, witness::PartialWitness},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{impl_target_set, Sha256TargetGoldilocks, SingleTarget, TargetSet},
        ExtendedComposeArgs, ProofComposition,
    },
    next_validator_set::{NextValidatorSet, NextValidatorSetTarget},
    prelude::{
        consts::{CIRCUIT_DIGEST_SIZE, SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS},
        *,
    },
    ProofWithCircuitData,
};

// TODO: Assert that contsants merkle caps are equal too.
impl_target_set! {
    pub struct RecursiveValidatorSetChangeTarget {
        prev_circuit_digest: HashOutTarget,
        validator_set_hash: Sha256TargetGoldilocks,
        authority_set_id: SingleTarget,
    }
}

#[derive(Clone)]
pub struct ValidatorSetGenesis {
    pub validator_set_hash: [u64; SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS],
    pub authority_set_id: u64,
}

impl ValidatorSetGenesis {
    pub fn prove(&self) -> ProofWithCircuitData<RecursiveValidatorSetChangeTarget> {
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_ecc_config());

        let placeholder_prev_circuit_digest = [F::ZERO; CIRCUIT_DIGEST_SIZE];
        let targets = builder.constants(&placeholder_prev_circuit_digest);
        builder.register_public_inputs(&targets);

        let targets = builder.constants(
            &self
                .validator_set_hash
                .into_iter()
                .map(|e| F::from_canonical_u64(e))
                .collect::<Vec<_>>(),
        );
        builder.register_public_inputs(&targets);

        let target = builder.constant(F::from_canonical_u64(self.authority_set_id));
        builder.register_public_input(target);

        let witness = PartialWitness::new();

        ProofWithCircuitData::from_builder(builder, witness)
    }
}

pub struct LatestValidatorSet {
    pub genesis_data: ValidatorSetGenesis,
    pub current_proof: ProofWithCircuitData<RecursiveValidatorSetChangeTarget>,
    pub change_proof: NextValidatorSet,
}

impl LatestValidatorSet {
    pub fn prove(self) -> ProofWithCircuitData<RecursiveValidatorSetChangeTarget> {
        log::info!("Appending validator set change proof to composition...");

        let next_validator_set_proof = self.change_proof.prove();
        // TODO: Cache it somehow?
        let genesis_proof_circuit_digest = self.genesis_data.prove().circuit_digest();

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let composition_builder =
            ProofComposition::new_with_config(next_validator_set_proof, self.current_proof, config);

        let targets_op =
            |builder: &mut CircuitBuilder<F, D>,
             ExtendedComposeArgs::<NextValidatorSetTarget, RecursiveValidatorSetChangeTarget> {
                 first_target_set: next_validator_set,
                 second_target_set: current_proof,
                 first_circuit_digest: _,
                 second_circuit_digest: current_proof_circuit_digest,
             }| {
                next_validator_set
                    .validator_set_hash
                    .connect(&current_proof.validator_set_hash, builder);

                current_proof
                    .authority_set_id
                    .connect(&next_validator_set.current_authority_set_id, builder);

                let current_proof_is_recursed = hashes_equal(
                    builder,
                    current_proof_circuit_digest,
                    current_proof.prev_circuit_digest,
                );

                let zeroed_circuit_digest = builder.constant_hash(HashOut::<F>::ZERO);
                let prev_circuit_digest_zeroed = hashes_equal(
                    builder,
                    zeroed_circuit_digest,
                    current_proof.prev_circuit_digest,
                );

                let genesis_circuit_digest = builder.constant_hash(genesis_proof_circuit_digest);
                let current_circuit_digest_is_genesis = hashes_equal(
                    builder,
                    genesis_circuit_digest,
                    current_proof_circuit_digest,
                );

                let current_proof_is_genesis = builder.and(
                    prev_circuit_digest_zeroed,
                    current_circuit_digest_is_genesis,
                );

                let recursion_correct =
                    or(builder, current_proof_is_recursed, current_proof_is_genesis);

                //builder.assert_one(recursion_correct.target);

                let one = builder.one();
                let next_authority_set_id =
                    builder.add(current_proof.authority_set_id.to_target(), one);
                let next_authority_set_id =
                    SingleTarget::parse(&mut std::iter::once(next_authority_set_id));

                RecursiveValidatorSetChangeTarget {
                    prev_circuit_digest: current_proof_circuit_digest,
                    validator_set_hash: next_validator_set.validator_set_hash,
                    authority_set_id: next_authority_set_id,
                }
            };

        composition_builder
            //.assert_first_circuit_digest()
            .extended_compose(targets_op)
    }
}

fn or(builder: &mut CircuitBuilder<F, D>, lhs: BoolTarget, rhs: BoolTarget) -> BoolTarget {
    let not_lhs = builder.not(lhs);
    let not_rhs = builder.not(rhs);
    let inv_res = builder.and(not_lhs, not_rhs);
    builder.not(inv_res)
}

fn hashes_equal(
    builder: &mut CircuitBuilder<F, D>,
    lhs: HashOutTarget,
    rhs: HashOutTarget,
) -> BoolTarget {
    lhs.elements
        .into_iter()
        .zip_eq(rhs.elements.into_iter())
        .map(|(el1, el2)| builder.is_equal(el1, el2))
        .collect::<Vec<_>>()
        .into_iter()
        .fold(builder._true(), |acc, elements_eq| {
            builder.and(acc, elements_eq)
        })
}
