//! ### Target definition that represents address in substrate storage.

use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::types::Field;
use plonky2_u32::gadgets::multiple_comparison::list_le_circuit;
use std::iter;

use crate::{
    common::{
        pad_byte_vec,
        targets::{impl_parsable_target_set, ArrayTarget, HalfByteTarget, TargetSet},
    },
    prelude::*,
};

pub const MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES: usize = 64;
pub const MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES: usize = MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2;

impl_parsable_target_set! {
    // Invariant: all the data after `length` is zeroed.
    pub struct StorageAddressTarget {
        /// Storage address padded with zeroes.
        pub padded_address: ArrayTarget<HalfByteTarget, MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES>,
        /// Length of address in nibbles.
        pub length: Target
    }
}

impl StorageAddressTarget {
    /// Create `StorageAddressTarget` with length = 0.
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        let zero = builder.zero();
        let mut nibbles = std::iter::repeat_n(zero, MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);

        Self {
            padded_address: ArrayTarget::parse_exact(&mut nibbles),
            length: builder.zero(),
        }
    }

    /// Create constant `StorageAddressTarget`.
    pub fn constant(nibbles: Vec<u8>, builder: &mut CircuitBuilder<F, D>) -> Self {
        assert!(nibbles.len() <= MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);

        let length = builder.constant(F::from_canonical_usize(nibbles.len()));

        let padded_address: [u8; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES] = pad_byte_vec(nibbles);
        let mut address = padded_address
            .into_iter()
            .map(|nibble| HalfByteTarget::constant(nibble, builder).to_target());
        let padded_address = ArrayTarget::parse_exact(&mut address);

        Self {
            padded_address,
            length,
        }
    }

    /// Create virtual `StorageAddressTarget` without inserting any checks on values.
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self::parse(&mut iter::repeat(()).map(|_| builder.add_virtual_target()))
    }

    /// Set witness value for `StorageAddressTarget`.
    pub fn set_witness(&self, nibbles: &[u8], witness: &mut PartialWitness<F>) {
        let length = nibbles.len();
        witness.set_target(self.length, F::from_canonical_usize(length));

        assert!(nibbles.len() <= MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);
        for (target, value) in self
            .padded_address
            .0
            .iter()
            .zip(nibbles.iter().copied().chain(iter::repeat(0)))
        {
            assert!(value < (1 << 4));
            witness.set_target(target.to_target(), F::from_canonical_u8(value));
        }
    }

    /// Create `StorageAddressTarget`.
    ///
    /// Note that targets after `self.length` won't be taken into account and will be zeroed in
    /// resulting `StorageAddressTarget`.
    pub fn from_half_byte_targets_safe(
        targets: [HalfByteTarget; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES],
        length: Target,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let zero = HalfByteTarget::constant(0, builder);
        let targets = targets
            .into_iter()
            .enumerate()
            .map(|(idx, unchecked_target)| {
                let idx_inc = builder.constant(F::from_canonical_usize(idx + 1));
                let valid_half_byte = list_le_circuit(builder, vec![idx_inc], vec![length], 32);

                // Returns `if b { x } else { y }`.
                let checked_target = builder.select(
                    valid_half_byte,
                    unchecked_target.to_target(),
                    zero.to_target(),
                );

                HalfByteTarget::from_target_unsafe(checked_target)
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Correct number of targets");

        Self {
            padded_address: ArrayTarget(targets),
            length,
        }
    }

    /// Creates `StorageAddressTarget` and sets only first nibble of it.
    pub fn from_single_nibble_target(
        nibble: HalfByteTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let zero = HalfByteTarget::constant(0, builder);
        let mut targets = [zero; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES];
        targets[0] = nibble;

        Self {
            padded_address: ArrayTarget(targets),
            length: builder.one(),
        }
    }

    /// Concatenate two `StorageAddressTarget`s.
    pub fn append(self, append: StorageAddressTarget, builder: &mut CircuitBuilder<F, D>) -> Self {
        let final_length = builder.add(self.length, append.length);
        let max_length = builder.constant(F::from_canonical_usize(
            MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES,
        ));
        let max_len_sub_final = builder.sub(max_length, final_length);
        // Assert that `max_len_sub_final` >= 0.
        // This works because `max_length`` << 2^32 and `final_length`` << 2^32.
        builder.range_check(max_len_sub_final, 32);

        let zero = builder.zero();
        let mut address_targets = (0..MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES).map(|i| {
            let self_read_idx = builder.constant(F::from_canonical_usize(i));
            let appended_read_idx = builder.sub(self_read_idx, self.length);

            // `appended_read_idx` is valid iff `self_read_idx` >= `self.length`.
            // Elsewhere we don't care about actual idx and set it to zero, as we don't use it's value.
            let appended_read_idx_valid =
                list_le_circuit(builder, vec![self.length], vec![self_read_idx], 32);
            // Returns `if b { x } else { y }`.
            let appended_read_idx =
                builder.select(appended_read_idx_valid, appended_read_idx, zero);

            let self_nibble = self.padded_address.constant_read(i);
            let appended_nibble = append
                .padded_address
                .random_read(appended_read_idx, builder);

            // Check if `self.length` <= `i`
            let select_nibble_from_appended =
                list_le_circuit(builder, vec![self.length], vec![self_read_idx], 32);

            // Returns `if b { x } else { y }`.
            builder.select(
                select_nibble_from_appended,
                appended_nibble.to_target(),
                self_nibble.to_target(),
            )
        });

        let padded_address = ArrayTarget::parse_exact(&mut address_targets);

        Self {
            padded_address,
            length: final_length,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{tests_common::create_address_target, *};
    use plonky2::plonk::{
        circuit_data::{CircuitConfig, CircuitData},
        proof::ProofWithPublicInputs,
    };

    #[test]
    fn test_address_append() {
        address_append_test_case(&[], &[], Some(&[]));

        address_append_test_case(&[0xA], &[0x3], Some(&[0xA, 0x3]));

        address_append_test_case(&[0x1, 0x2, 0x3, 0x4], &[], Some(&[0x1, 0x2, 0x3, 0x4]));

        address_append_test_case(
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            Some(&[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES]),
        );
    }

    #[test]
    #[should_panic(
        expected = "assertion `left == right` failed: Partition containing Wire(Wire { row: 130, column: 33 }) was set twice with different values: 0 != 1"
    )]
    fn test_address_append_overflow_fails() {
        address_append_test_case(&[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES], &[0x1], None);
    }

    #[test]
    fn test_address_append_have_constant_verifier_data() {
        let (first_cd, _) = build_test_case_circuit(&[0xA], &[0x3], Some(&[0xA, 0x3]));
        let (second_cd, _) = build_test_case_circuit(
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            Some(&[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES]),
        );

        assert_eq!(first_cd.verifier_only, second_cd.verifier_only);
    }

    fn address_append_test_case(
        first_nibbles: &[u8],
        second_nibbles: &[u8],
        expected_nibbles: Option<&[u8]>,
    ) {
        let (circuit, proof) =
            build_test_case_circuit(first_nibbles, second_nibbles, expected_nibbles);
        circuit.verify(proof).expect("Failed to verify");
    }

    fn build_test_case_circuit(
        first_nibbles: &[u8],
        second_nibbles: &[u8],
        expected_nibbles: Option<&[u8]>,
    ) -> (CircuitData<F, C, D>, ProofWithPublicInputs<F, C, D>) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let first_address = create_address_target(first_nibbles, &mut builder, &mut pw);
        let second_address = create_address_target(second_nibbles, &mut builder, &mut pw);

        let result = first_address.append(second_address, &mut builder);

        if let Some(nibbles) = expected_nibbles {
            let address = create_address_target(nibbles, &mut builder, &mut pw);
            result.connect(&address, &mut builder);
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");

        (circuit, proof)
    }

    #[test]
    fn test_address_from_half_byte_targets_safe() {
        address_from_half_byte_targets_safe_test_case(&pad_byte_vec(vec![]), 0, &[]);

        address_from_half_byte_targets_safe_test_case(
            &pad_byte_vec(vec![1, 2, 3, 4]),
            4,
            &[1, 2, 3, 4],
        );

        address_from_half_byte_targets_safe_test_case(
            &pad_byte_vec(vec![1, 2, 3, 4, 5]),
            3,
            &[1, 2, 3],
        );
    }

    fn address_from_half_byte_targets_safe_test_case(
        data: &[u8; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES],
        length: usize,
        expected_data: &[u8],
    ) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let targets = data
            .iter()
            .map(|byte| HalfByteTarget::constant(*byte, &mut builder))
            .collect::<Vec<_>>()
            .try_into()
            .expect("Correct length of data");

        let length = builder.constant(F::from_canonical_usize(length));

        let address =
            StorageAddressTarget::from_half_byte_targets_safe(targets, length, &mut builder);

        let expected_address = create_address_target(expected_data, &mut builder, &mut pw);

        address.register_as_public_inputs(&mut builder);

        address.connect(&expected_address, &mut builder);

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}

#[cfg(test)]
pub mod tests_common {
    use super::*;

    pub fn create_address_target(
        nibbles: &[u8],
        builder: &mut CircuitBuilder<F, D>,
        witness: &mut PartialWitness<F>,
    ) -> StorageAddressTarget {
        assert!(nibbles.len() <= MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);
        let length = builder.add_virtual_target();
        witness.set_target(length, F::from_canonical_usize(nibbles.len()));

        let mut nibble_targets = nibbles
            .iter()
            .cloned()
            .chain(iter::repeat(0))
            .take(MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES)
            .map(|nibble| {
                let nibble_target = builder.add_virtual_target();
                witness.set_target(nibble_target, F::from_canonical_u8(nibble));
                HalfByteTarget::from_target_safe(nibble_target, builder).to_target()
            });

        StorageAddressTarget {
            length,
            padded_address: ArrayTarget::parse_exact(&mut nibble_targets),
        }
    }
}
