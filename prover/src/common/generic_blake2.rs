//! ### Contains circuit that's used to compute blake2 hash of generic-length data.

use lazy_static::lazy_static;
use plonky2::{
    gates::noop::NoopGate,
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, VerifierOnlyCircuitData},
    },
};
use plonky2_blake2b256::circuit::{
    blake2_circuit_from_message_targets_and_length_target, BLOCK_BITS, BLOCK_BYTES,
};
use plonky2_field::types::Field;

use crate::{
    common::{
        targets::{ArrayTarget, Blake2Target, ByteTarget, TargetSet},
        ProofWithCircuitData,
    },
    prelude::*,
};

use super::pad_byte_vec;

/// Maximum amount of blake2 blocks.
const MAX_BLOCK_COUNT: usize = 8;
/// Max data length that this circuit will accept.
pub const MAX_DATA_BYTES: usize = MAX_BLOCK_COUNT * BLOCK_BYTES;

impl_parsable_target_set! {
    /// Public inputs for `GenericBlake2`.
    pub struct GenericBlake2Target {
        /// It's guaranteed that padding of data will be zeroed and asserted to be equal 0.
        pub data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        /// Length of a useful data.
        pub length: Target,
        /// Resulting hash.
        pub hash: Blake2Target
    }
}

// Unlike `VariativeBlake2`, this circuit will have constant `VerifierOnlyCircuitData` across all
// the valid inputs.
pub struct GenericBlake2 {
    /// Data to be hashed.
    data: Vec<u8>,
}

impl GenericBlake2 {
    /// Create new `GenericBlake2` circuit.
    ///
    /// This function will statically check that `MAX_DATA_LENGTH_ESTIMATION`
    /// don't exceed `MAX_DATA_BYTES`.
    pub fn new<const MAX_DATA_LENGTH_ESTIMATION: usize>(data: Vec<u8>) -> Self {
        #[allow(clippy::let_unit_value)]
        let _ = AssertDataLengthValid::<MAX_DATA_LENGTH_ESTIMATION>::VALID;

        assert!(
            data.len() <= MAX_DATA_LENGTH_ESTIMATION,
            "data.len() = {}, MAX_DATA_LENGTH_ESTIMATION = {MAX_DATA_LENGTH_ESTIMATION}",
            data.len()
        );

        Self { data }
    }
}

struct AssertDataLengthValid<const DATA_LENGTH: usize>;

impl<const DATA_LENGTH: usize> AssertDataLengthValid<DATA_LENGTH> {
    const VALID: () = assert!(DATA_LENGTH <= MAX_DATA_BYTES);
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
            verifier_data_targets.push(
                verifier_data_targets
                    .last()
                    .expect("VERIFIER_DATA_BY_BLOCK_COUNT must be >= 1")
                    .clone(),
            );
        }

        // It's ok not to check `verifier_data_idx` range as `GenericBlake2` just exposes all the
        // public inputs of `VariativeBlake2`, so we need to check just that it's contained in
        // pre-computed verifier data array. All the other assertions must be performed in
        // `VariativeBlake2`.
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

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}

lazy_static! {
    /// Cached `VerifierOnlyCircuitData`s, each corresponding to a specific blake2 block count.
    static ref VERIFIER_DATA_BY_BLOCK_COUNT: [VerifierOnlyCircuitData<C, D>; MAX_BLOCK_COUNT] = (1
        ..=MAX_BLOCK_COUNT)
        .map(blake2_circuit_verifier_data)
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct max block count");
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
        length: Target,
        hash: Blake2Target
    }
}

/// Inner circuit that will have different `VerifierOnlyCircuitData` for each block count.
/// This circuit asserts that data padding is zeroed(it applies to targets, not the `data` field).
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

        let data_target: [ByteTarget; MAX_DATA_BYTES] = pad_byte_vec(self.data).map(|byte| {
            let target = builder.add_virtual_target();
            witness.set_target(target, F::from_canonical_u8(byte));
            ByteTarget::from_target_safe(target, &mut builder)
        });

        // Assert that padding is zeroed.
        let mut data_end = builder._false();
        let mut current_idx = builder.zero();
        let zero = builder.zero();
        for byte in data_target.iter().take(block_count * BLOCK_BYTES) {
            let len_exceeded = builder.is_equal(current_idx, length_target);
            data_end = builder.or(len_exceeded, data_end);

            let byte_is_zero = builder.is_equal(byte.as_target(), zero);
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
            .flat_map(|t| t.as_bit_targets(&mut builder).0.into_iter().rev());

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
            length: length_target,
            hash,
        }
        .register_as_public_inputs(&mut builder);

        // Standardize degree.
        while builder.num_gates() < 1 << 16 {
            builder.add_gate(NoopGate, vec![]);
        }

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}

#[cfg(test)]
mod tests {
    use blake2::{
        digest::{Update, VariableOutput},
        Blake2bVar,
    };
    use plonky2::plonk::circuit_data::VerifierCircuitData;

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
        let mut hasher = Blake2bVar::new(32).expect("Blake2bVar instantiated");
        hasher.update(&data);
        let mut real_hash = [0; 32];
        hasher
            .finalize_variable(&mut real_hash)
            .expect("Hash of correct length");

        let proof = GenericBlake2 { data: data.clone() }.prove();
        let public_inputs =
            GenericBlake2Target::parse_public_inputs_exact(&mut proof.public_inputs().into_iter());

        assert_eq!(public_inputs.hash.to_vec(), array_to_bits(&real_hash));
        assert_eq!(public_inputs.length as usize, data.len());
        assert_eq!(&public_inputs.data[..data.len()], &data[..]);

        proof.circuit_data().clone()
    }
}
