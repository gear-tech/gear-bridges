use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_ed25519::gadgets::eddsa::ed25519_circuit;
use plonky2_field::types::Field;

use crate::{
    common::{array_to_bits, ProofCompositionBuilder, ProofCompositionTargets},
    prelude::*,
    validator_set_hash::ValidatorSetHash,
    ProofWithCircuitData,
};

const PUBLIC_KEY_SIZE: usize = 32;
const PUBLIC_KEY_SIZE_IN_BITS: usize = PUBLIC_KEY_SIZE * 8;

const SIGNATURE_SIZE: usize = 64;

const SHA256_DIGEST_SIZE: usize = 32;
const SHA256_DIGEST_SIZE_IN_BITS: usize = SHA256_DIGEST_SIZE * 8;

#[derive(Clone)]
pub struct PreCommit {
    pub public_key: [u8; PUBLIC_KEY_SIZE],
    pub signature: [u8; SIGNATURE_SIZE],
}

/// Public inputs:
/// - validator set hash
/// - message
#[derive(Clone)]
pub struct BlockFinality {
    pub validator_set: Vec<[u8; PUBLIC_KEY_SIZE]>,
    pub pre_commits: Vec<PreCommit>,
    pub message: Vec<u8>,
}

impl BlockFinality {
    pub fn prove(&self) -> ProofWithCircuitData {
        let processed_pre_commits = self
            .pre_commits
            .iter()
            .map(|pc| ProcessedPreCommit {
                validator_idx: self
                    .validator_set
                    .iter()
                    .position(|v| v == &pc.public_key)
                    .unwrap(),
                signature: pc.signature,
            })
            .collect();

        let validator_set_hash_proof = ValidatorSetHash {
            validator_set: self.validator_set.clone(),
        }
        .prove();

        let validator_signs_proof = ValidatorSignsChain {
            validator_set: self.validator_set.clone(),
            pre_commits: processed_pre_commits,
            message: self.message.clone(),
        }
        .prove();

        let composition_builder =
            ProofCompositionBuilder::new(validator_set_hash_proof, validator_signs_proof);

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let validator_set_hash_public_inputs = targets.first_proof_public_input_targets;
            let validator_signs_public_inputs = targets.second_proof_public_input_targets;

            // Set validator set hash as public input.
            for target in &validator_set_hash_public_inputs[..SHA256_DIGEST_SIZE_IN_BITS] {
                builder.register_public_input(*target);
            }

            // Set message as public input.
            for target in &validator_signs_public_inputs[1 + self.validator_set.len()
                * PUBLIC_KEY_SIZE_IN_BITS
                ..1 + self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS + self.message.len() * 8]
            {
                builder.register_public_input(*target);
            }

            // Assert that validator sets are matching.
            let validator_set_targets_0 = &validator_set_hash_public_inputs
                [SHA256_DIGEST_SIZE_IN_BITS
                    ..SHA256_DIGEST_SIZE_IN_BITS
                        + self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS];
            let validator_set_targets_1 = &validator_signs_public_inputs
                [1..1 + self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS];
            for (target_0, target_1) in validator_set_targets_0
                .iter()
                .zip(validator_set_targets_1.iter())
            {
                builder.connect(*target_0, *target_1);
            }
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}

#[derive(Clone)]
struct ProcessedPreCommit {
    validator_idx: usize,
    signature: [u8; SIGNATURE_SIZE],
}

/// Public inputs order:
/// - latest proven index
/// - validator set
/// - message
struct ValidatorSignsChain {
    validator_set: Vec<[u8; PUBLIC_KEY_SIZE]>,
    pre_commits: Vec<ProcessedPreCommit>,
    message: Vec<u8>,
}

impl ValidatorSignsChain {
    pub fn prove(&self) -> ProofWithCircuitData {
        let mut pre_commits = self.pre_commits.clone();
        pre_commits.sort_by(|a, b| a.validator_idx.cmp(&b.validator_idx));

        pre_commits
            .iter()
            .map(|pre_commit| {
                IndexedValidatorSign {
                    validator_set: self.validator_set.clone(),
                    index: pre_commit.validator_idx,
                    signature: pre_commit.signature,
                    message: self.message.clone(),
                }
                .prove()
            })
            .reduce(|acc, x| {
                ComposedValidatorSigns {}.prove(
                    acc,
                    x,
                    self.message.len() * 8,
                    self.validator_set.len(),
                )
            })
            .unwrap()
    }
}

/// Public inputs order:
/// - latest proven index
/// - validator set
/// - message
struct ComposedValidatorSigns {}

impl ComposedValidatorSigns {
    fn prove(
        &self,
        previous_composed_proof: ProofWithCircuitData,
        indexed_sign_proof: ProofWithCircuitData,
        message_size_in_bits: usize,
        validator_set_length: usize,
    ) -> ProofWithCircuitData {
        let composition_builder =
            ProofCompositionBuilder::new(previous_composed_proof, indexed_sign_proof);

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let previous_composed_proof_public_inputs = targets.first_proof_public_input_targets;
            let indexed_sign_proof_public_inputs = targets.second_proof_public_input_targets;

            // Set latest proven index as public input.
            builder.register_public_input(indexed_sign_proof_public_inputs[0]);

            // Set validator set and message as public inputs.
            for target in &indexed_sign_proof_public_inputs
                [1..1 + message_size_in_bits + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS]
            {
                builder.register_public_input(*target);
            }

            // Assert that messages are matching.
            let message_targets_composed = previous_composed_proof_public_inputs[1
                + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS
                ..1 + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS + message_size_in_bits]
                .iter();
            let message_targets_new = indexed_sign_proof_public_inputs[1 + validator_set_length
                * PUBLIC_KEY_SIZE_IN_BITS
                ..1 + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS + message_size_in_bits]
                .iter();
            for (target_0, target_1) in message_targets_composed.zip(message_targets_new) {
                builder.connect(*target_0, *target_1);
            }

            // Assert that validator set is matching.
            let message_targets_composed = previous_composed_proof_public_inputs
                [1..1 + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS]
                .iter();
            let message_targets_new = indexed_sign_proof_public_inputs
                [1..1 + validator_set_length * PUBLIC_KEY_SIZE_IN_BITS]
                .iter();
            for (target_0, target_1) in message_targets_composed.zip(message_targets_new) {
                builder.connect(*target_0, *target_1);
            }

            // Assert that newly proven index > latest proven.
            let new_index_sub_latest = builder.sub(
                indexed_sign_proof_public_inputs[0],
                previous_composed_proof_public_inputs[0],
            );
            let one = builder.one();
            let to_compare_with_0 = builder.sub(new_index_sub_latest, one); // assert >= 0.
            builder.range_check(to_compare_with_0, 32);
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}

/// Public inputs order:
/// - index
/// - validator set
/// - message
struct IndexedValidatorSign {
    validator_set: Vec<[u8; PUBLIC_KEY_SIZE]>,
    index: usize,
    message: Vec<u8>,
    signature: [u8; SIGNATURE_SIZE],
}

impl IndexedValidatorSign {
    fn prove(&self) -> ProofWithCircuitData {
        let selector_proof = ValidatorSelector {
            validator_set: self.validator_set.clone(),
            index: self.index,
            validator: self.validator_set[self.index],
        }
        .prove();

        let sign_proof = SingleValidatorSign {
            public_key: self.validator_set[self.index],
            signature: self.signature,
            message: self.message.clone(),
        }
        .prove();

        let composition_builder = ProofCompositionBuilder::new(selector_proof, sign_proof);

        let targets_op = |builder: &mut CircuitBuilder<F, D>, targets: ProofCompositionTargets| {
            let selector_proof_public_inputs = targets.first_proof_public_input_targets;
            let sign_proof_public_inputs = targets.second_proof_public_input_targets;

            // Set index and validator set as public inputs.
            for target in &selector_proof_public_inputs
                [..1 + self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS]
            {
                builder.register_public_input(*target);
            }

            // Set message as public input.
            for target in &sign_proof_public_inputs[..self.message.len() * 8] {
                builder.register_public_input(*target);
            }

            // Assert that validator that signed message is at correct index.
            let selector_proof_validator_targets = &selector_proof_public_inputs[1 + self
                .validator_set
                .len()
                * PUBLIC_KEY_SIZE_IN_BITS
                ..1 + self.validator_set.len() * PUBLIC_KEY_SIZE_IN_BITS + PUBLIC_KEY_SIZE_IN_BITS];
            let sign_proof_validator_targets = &sign_proof_public_inputs
                [self.message.len() * 8..self.message.len() * 8 + PUBLIC_KEY_SIZE_IN_BITS];

            for (target_0, target_1) in selector_proof_validator_targets
                .iter()
                .zip(sign_proof_validator_targets.iter())
            {
                builder.connect(*target_0, *target_1);
            }
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}

/// Public inputs order:
/// - message
/// - public key
/// - signature
struct SingleValidatorSign {
    public_key: [u8; PUBLIC_KEY_SIZE],
    signature: [u8; SIGNATURE_SIZE],
    message: Vec<u8>,
}

impl SingleValidatorSign {
    fn prove(&self) -> ProofWithCircuitData {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        let targets = ed25519_circuit(&mut builder, self.message.len() * 8);

        for target in &targets.msg {
            builder.register_public_input(target.target);
        }

        for target in &targets.pk {
            builder.register_public_input(target.target);
        }

        for target in &targets.sig {
            builder.register_public_input(target.target);
        }

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

        ProofWithCircuitData::from_builder(builder, pw)
    }
}

/// Public inputs order:
/// - index
/// - validator set
/// - validator
struct ValidatorSelector {
    validator_set: Vec<[u8; PUBLIC_KEY_SIZE]>,
    index: usize,
    validator: [u8; PUBLIC_KEY_SIZE],
}

impl ValidatorSelector {
    fn prove(&self) -> ProofWithCircuitData {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        let targets = validator_selector_circuit(&mut builder, self.validator_set.len());

        builder.register_public_input(targets.index);

        for target in targets.validator_set.iter().flatten() {
            builder.register_public_input(target.target);
        }

        for target in &targets.validator {
            builder.register_public_input(target.target);
        }

        let mut pw = PartialWitness::new();

        pw.set_target(targets.index, F::from_canonical_u32(self.index as u32));

        let validator_set_bits = self.validator_set.iter().flat_map(|v| array_to_bits(v));
        for (target, value) in targets
            .validator_set
            .iter()
            .flatten()
            .zip(validator_set_bits)
        {
            pw.set_bool_target(*target, value);
        }

        let validator_bits = array_to_bits(&self.validator).into_iter();
        for (target, value) in targets.validator.iter().zip(validator_bits) {
            pw.set_bool_target(*target, value);
        }

        ProofWithCircuitData::from_builder(builder, pw)
    }
}

struct ValidatorSelectorTargets {
    validator_set: Vec<[BoolTarget; PUBLIC_KEY_SIZE_IN_BITS]>,
    index: Target,
    validator: [BoolTarget; PUBLIC_KEY_SIZE_IN_BITS],
}

fn validator_selector_circuit(
    builder: &mut CircuitBuilder<F, D>,
    validator_count: usize,
) -> ValidatorSelectorTargets {
    let mut validator_set_targets = Vec::with_capacity(validator_count);
    for _ in 0..validator_count {
        let pk_targets: [BoolTarget; PUBLIC_KEY_SIZE_IN_BITS] = (0..PUBLIC_KEY_SIZE_IN_BITS)
            .map(|_| builder.add_virtual_bool_target_safe())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        validator_set_targets.push(pk_targets);
    }

    let index_target = builder.add_virtual_target();

    let mut equality_targets = Vec::with_capacity(validator_count);
    for i in 0..validator_count {
        let i_target = builder.constant(F::from_canonical_u32(i as u32));
        let eq_target = builder.is_equal(i_target, index_target);
        equality_targets.push(eq_target);
    }

    let mut validator_targets = Vec::with_capacity(PUBLIC_KEY_SIZE_IN_BITS);
    for bit_idx in 0..PUBLIC_KEY_SIZE_IN_BITS {
        let target = validator_set_targets
            .iter()
            .zip(equality_targets.iter())
            .map(|(validator, equality)| builder.and(validator[bit_idx], *equality))
            .collect::<Vec<_>>()
            .into_iter()
            .reduce(|acc, x| builder.or(acc, x))
            .unwrap();

        validator_targets.push(target);
    }

    ValidatorSelectorTargets {
        validator_set: validator_set_targets,
        index: index_target,
        validator: validator_targets.try_into().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_selector() {
        let validator_selector = ValidatorSelector {
            validator_set: vec![
                [10; PUBLIC_KEY_SIZE],
                [20; PUBLIC_KEY_SIZE],
                [11; PUBLIC_KEY_SIZE],
                [193; PUBLIC_KEY_SIZE],
            ],
            index: 2,
            validator: [11; PUBLIC_KEY_SIZE],
        };
        let proof = validator_selector.prove();

        assert!(proof.verify());
    }

    #[test]
    #[should_panic]
    fn test_incorrect_validator_selector_fails() {
        let validator_selector = ValidatorSelector {
            validator_set: vec![
                [0; PUBLIC_KEY_SIZE],
                [0; PUBLIC_KEY_SIZE],
                [1; PUBLIC_KEY_SIZE],
                [0; PUBLIC_KEY_SIZE],
            ],
            index: 2,
            validator: [0; PUBLIC_KEY_SIZE],
        };
        validator_selector.prove();
    }
}
