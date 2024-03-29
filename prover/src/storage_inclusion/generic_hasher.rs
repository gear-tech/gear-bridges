use lazy_static::lazy_static;
use plonky2::{
    gates::noop::NoopGate,
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, VerifierOnlyCircuitData},
    },
};
use plonky2_blake2b256::circuit::{
    blake2_circuit_from_message_targets_and_length_target, BLOCK_BITS, BLOCK_BYTES,
};
use plonky2_field::types::Field;
use std::iter;

use crate::{
    common::targets::{
        impl_parsable_target_set, impl_target_set, ArrayTarget, Blake2Target, ByteTarget,
        SingleTarget, TargetSet,
    },
    prelude::*,
    ProofWithCircuitData,
};

const MAX_BLOCK_COUNT: usize = 8;
pub const MAX_DATA_BYTES: usize = MAX_BLOCK_COUNT * BLOCK_BYTES;

impl_parsable_target_set! {
    pub struct GenericBlake2Target {
        pub data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        pub length: SingleTarget,
        pub hash: Blake2Target
    }
}

pub struct GenericBlake2 {
    pub data: Vec<u8>,
}

impl GenericBlake2 {
    pub fn prove(self) -> ProofWithCircuitData<GenericBlake2Target> {
        let block_count = self.data.len().div_ceil(BLOCK_BYTES).max(1);
        assert!(block_count <= MAX_BLOCK_COUNT);

        let variative_proof = VariativeBlake2 { data: self.data }.prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let block_count_target = builder.add_virtual_target();
        witness.set_target(block_count_target, F::from_canonical_usize(block_count));

        let proof_with_pis_target =
            builder.add_virtual_proof_with_pis(&variative_proof.circuit_data().common);

        let mut verifier_data_targets = VERIFIER_DATA_BY_BLOCK_COUNT
            .iter()
            .map(|verifier_data| builder.constant_verifier_data(verifier_data))
            .collect::<Vec<_>>();
        for _ in verifier_data_targets.len()..verifier_data_targets.len().next_power_of_two() {
            verifier_data_targets.push(verifier_data_targets.last().unwrap().clone());
        }

        let verifier_data_idx = builder.add_const(block_count_target, F::NEG_ONE);
        let verifier_data_target =
            builder.random_access_verifier_data(verifier_data_idx, verifier_data_targets);

        witness.set_proof_with_pis_target(&proof_with_pis_target, &variative_proof.proof());
        builder.verify_proof::<C>(
            &proof_with_pis_target,
            &verifier_data_target,
            &variative_proof.circuit_data().common,
        );

        let inner_pis = VariativeBlake2Target::parse_exact(
            &mut proof_with_pis_target.public_inputs.into_iter(),
        );

        GenericBlake2Target {
            data: inner_pis.data,
            length: inner_pis.length,
            hash: inner_pis.hash,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}

lazy_static! {
    static ref VERIFIER_DATA_BY_BLOCK_COUNT: [VerifierOnlyCircuitData<C, D>; MAX_BLOCK_COUNT] = (1
        ..=MAX_BLOCK_COUNT)
        .map(blake2_circuit_verifier_data)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
}

fn blake2_circuit_verifier_data(num_blocks: usize) -> VerifierOnlyCircuitData<C, D> {
    VariativeBlake2 {
        data: vec![0; BLOCK_BYTES * num_blocks],
    }
    .prove()
    .circuit_data()
    .verifier_only
    .clone()
}

impl_target_set! {
    struct VariativeBlake2Target {
        data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        length: SingleTarget,
        hash: Blake2Target
    }
}

struct VariativeBlake2 {
    data: Vec<u8>,
}

impl VariativeBlake2 {
    pub fn prove(self) -> ProofWithCircuitData<VariativeBlake2Target> {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let block_count = self.data.len().div_ceil(BLOCK_BYTES).max(1);

        let length_target = builder.add_virtual_target();
        witness.set_target(length_target, F::from_canonical_usize(self.data.len()));

        let data_target: [ByteTarget; MAX_DATA_BYTES] = self
            .data
            .into_iter()
            .chain(iter::repeat(0))
            .take(MAX_DATA_BYTES)
            .map(|byte| {
                let target = builder.add_virtual_target();
                witness.set_target(target, F::from_canonical_u8(byte));
                ByteTarget::from_target_safe(target, &mut builder)
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        // Assert that padding is zeroed.
        let mut data_end = builder._false();
        let mut current_idx = builder.zero();
        let zero = builder.zero();
        for byte in data_target.iter().take(block_count * BLOCK_BYTES) {
            let len_exceeded = builder.is_equal(current_idx, length_target);
            data_end = builder.or(len_exceeded, data_end);

            let byte_is_zero = builder.is_equal(byte.to_target(), zero);
            let byte_is_not_zero = builder.not(byte_is_zero);

            let byte_invalid = builder.and(data_end, byte_is_not_zero);
            builder.assert_zero(byte_invalid.target);

            current_idx = builder.add_const(current_idx, F::ONE);
        }

        // Assert upper bound for length.
        let length_is_max = builder.is_equal(current_idx, length_target);
        let length_valid = builder.or(length_is_max, data_end);
        builder.assert_one(length_valid.target);

        // Assert lower bound for length.
        let max_length = builder.constant(F::from_canonical_usize(block_count * BLOCK_BYTES));
        let padded_length = builder.sub(max_length, length_target);
        let block_bytes_target = builder.constant(F::from_canonical_usize(BLOCK_BYTES));
        let compare_with_zero = builder.sub(block_bytes_target, padded_length);
        builder.range_check(compare_with_zero, 32);

        let data_target = ArrayTarget(data_target);
        let data_target_bits = data_target
            .0
            .iter()
            .flat_map(|t| t.to_bit_targets(&mut builder).0.into_iter().rev());

        let hasher_input = data_target_bits
            .take(BLOCK_BITS * block_count)
            .collect::<Vec<_>>();

        let hash = blake2_circuit_from_message_targets_and_length_target(
            &mut builder,
            hasher_input,
            length_target,
        );
        let hash = Blake2Target::parse_exact(&mut hash.into_iter().map(|t| t.target));

        VariativeBlake2Target {
            data: data_target,
            length: length_target.into(),
            hash,
        }
        .register_as_public_inputs(&mut builder);

        // Standardize degree.
        while builder.num_gates() < 1 << 16 {
            builder.add_gate(NoopGate, vec![]);
        }

        ProofWithCircuitData::from_builder(builder, witness)
    }
}

#[cfg(test)]
mod tests {
    use blake2::{
        digest::{Update, VariableOutput},
        Blake2bVar,
    };
    use plonky2::plonk::circuit_data::VerifierCircuitData;
    use plonky2_blake2b256::circuit::BLOCK_BYTES;

    use super::*;
    use crate::common::{array_to_bits, targets::ParsableTargetSet};

    #[test]
    fn test_generic_blake2_hasher() {
        let test_data = vec![
            vec![0],
            vec![],
            vec![1, 3, 7, 11, 200, 103, 255, 0, 11],
            vec![10; BLOCK_BYTES - 1],
            vec![10; BLOCK_BYTES],
            vec![10; BLOCK_BYTES + 1],
            vec![0xA; BLOCK_BYTES * MAX_BLOCK_COUNT - 1],
            vec![0xA; BLOCK_BYTES * MAX_BLOCK_COUNT],
        ];

        let verifier_data = test_data.into_iter().map(test_case).collect::<Vec<_>>();

        for i in 1..verifier_data.len() {
            assert_eq!(
                verifier_data[i - 1],
                verifier_data[i],
                "Verifier data at {} and {} don't match",
                i - 1,
                i
            );
        }
    }

    fn test_case(data: Vec<u8>) -> VerifierCircuitData<F, C, D> {
        let mut hasher = Blake2bVar::new(32).unwrap();
        hasher.update(&data);
        let mut real_hash = [0; 32];
        hasher.finalize_variable(&mut real_hash).unwrap();

        let proof = GenericBlake2 { data: data.clone() }.prove();
        let public_inputs =
            GenericBlake2Target::parse_public_inputs_exact(&mut proof.pis().into_iter());

        assert_eq!(public_inputs.hash.to_vec(), array_to_bits(&real_hash));
        assert_eq!(public_inputs.length as usize, data.len());
        assert_eq!(&public_inputs.data[..data.len()], &data[..]);

        proof.circuit_data().clone()
    }
}
