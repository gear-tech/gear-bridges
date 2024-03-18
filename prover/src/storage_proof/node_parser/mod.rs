use std::iter;

use itertools::Itertools;
use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::types::{Field, PrimeField64};
use plonky2_u32::gadgets::multiple_comparison::list_le_circuit;

use crate::{
    common::targets::{
        impl_array_target_wrapper, impl_target_set, ArrayTarget, ByteTarget, HalfByteTarget,
        ParsableTargetSet, SingleTarget, TargetSet,
    },
    prelude::*,
};

mod bitmap_parser;
mod branch_header_parser;
mod child_node_array_parser;
mod nibble_parser;

const NODE_DATA_BLOCK_BYTES: usize = 128;
const MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS: usize = 5;
const MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES: usize = 64;
const MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES: usize = MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2;

impl_array_target_wrapper!(NodeDataBlockTarget, ByteTarget, NODE_DATA_BLOCK_BYTES);

impl ParsableTargetSet for NodeDataBlockTarget {
    type PublicInputsData = [u8; NODE_DATA_BLOCK_BYTES];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .take(NODE_DATA_BLOCK_BYTES)
            .map(|pi| pi.to_canonical_u64() as u8)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}

impl NodeDataBlockTarget {
    pub fn constant(
        data: &[u8; NODE_DATA_BLOCK_BYTES],
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let mut data_targets = data
            .iter()
            .map(|byte| builder.constant(F::from_canonical_u8(*byte)));
        Self::parse_exact(&mut data_targets)
    }

    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Self {
        let mut targets = (0..NODE_DATA_BLOCK_BYTES).map(|_| builder.add_virtual_target());
        Self::parse_exact(&mut targets)
    }

    pub fn set_witness(&self, data: &[u8; NODE_DATA_BLOCK_BYTES], witness: &mut PartialWitness<F>) {
        self.0
             .0
            .iter()
            .zip_eq(data.iter())
            .for_each(|(target, value)| {
                witness.set_target(target.to_target(), F::from_canonical_u8(*value))
            });
    }
}

impl_array_target_wrapper!(
    BranchNodeDataPaddedTarget,
    NodeDataBlockTarget,
    MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS
);

impl ParsableTargetSet for BranchNodeDataPaddedTarget {
    type PublicInputsData = [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        (0..MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS)
            .map(|_| NodeDataBlockTarget::parse_public_inputs(public_inputs))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}

impl BranchNodeDataPaddedTarget {
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> BranchNodeDataPaddedTarget {
        let targets = (0..MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS)
            .map(|_| NodeDataBlockTarget::add_virtual_unsafe(builder))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        Self(ArrayTarget(targets))
    }

    pub fn set_witness(
        &self,
        data: &[[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
        witness: &mut PartialWitness<F>,
    ) {
        self.0
             .0
            .iter()
            .zip_eq(data.into_iter())
            .for_each(|(target, data)| target.set_witness(data, witness));
    }

    pub fn random_read_array<const L: usize>(
        &self,
        at: SingleTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> ArrayTarget<ByteTarget, L> {
        let targets = (0..L)
            .map(|offset| {
                let offset = builder.constant(F::from_canonical_usize(offset));
                let read_at = builder.add(at.to_target(), offset);
                self.random_read(read_at.into(), builder)
            })
            .collect::<Vec<_>>();

        ArrayTarget(targets.try_into().unwrap())
    }

    pub fn random_read(&self, at: SingleTarget, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        let block_size = builder.constant(F::from_canonical_usize(NODE_DATA_BLOCK_BYTES));
        let max_data_size = builder.constant(F::from_canonical_usize(
            NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS,
        ));

        let shifted_block_size = builder.add(block_size, max_data_size);
        let mut current_offset = at.to_target();
        let mut block_already_selected = builder._false();
        let mut final_data = builder.zero();
        for block in &self.0 .0 {
            // Check that `current_offset` < `block_size`. This check is performed as:
            // `current_offset` + `max_data_size` + 1 <= `block_size` + `max_data_size`
            // to avoid negative numbers.
            let one = builder.one();
            let shifted_current_offset = builder.add_many(vec![current_offset, max_data_size, one]);
            let current_offset_eligible = list_le_circuit(
                builder,
                vec![shifted_current_offset],
                vec![shifted_block_size],
                32,
            );

            let block_not_yet_selected = builder.not(block_already_selected);
            let read_from_current_block =
                builder.and(current_offset_eligible, block_not_yet_selected);

            block_already_selected = builder.or(read_from_current_block, block_already_selected);

            let zero = builder.zero();

            // Returns `if b { x } else { y }`.
            // If we don't select from current block then we don't care about actual data that's read.
            let read_from = builder.select(read_from_current_block, current_offset, zero);
            let read_data = block.random_read(read_from.into(), builder);

            let masked_read_data =
                builder.mul(read_data.to_target(), read_from_current_block.target);
            final_data = builder.add(final_data, masked_read_data);

            current_offset = builder.sub(current_offset, block_size);
        }

        ByteTarget::from_target_unsafe(final_data)
    }
}

impl_array_target_wrapper!(
    StorageAddressPaddedTarget,
    HalfByteTarget,
    MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES
);

impl StorageAddressPaddedTarget {
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        let zero = builder.zero();
        let mut nibbles = iter::repeat(zero).take(MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);
        Self::parse_exact(&mut nibbles)
    }
}

impl_target_set! {
    // Invariant: all the data after `length` is zeroed.
    pub struct PartialStorageAddressTarget {
        pub address: StorageAddressPaddedTarget,
        pub length: SingleTarget
    }
}

impl PartialStorageAddressTarget {
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self {
            address: StorageAddressPaddedTarget::empty(builder),
            length: builder.zero().into(),
        }
    }

    /// Preserves invariant even if half-bytes after `self.length` contain trash.
    pub fn from_half_byte_targets_safe(
        targets: [HalfByteTarget; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES],
        length: SingleTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let zero = HalfByteTarget::constant(0, builder);
        let targets = targets
            .into_iter()
            .enumerate()
            .map(|(idx, unchecked_target)| {
                let idx_inc = builder.constant(F::from_canonical_usize(idx + 1));
                let valid_half_byte =
                    list_le_circuit(builder, vec![idx_inc], vec![length.to_target()], 32);

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
            .unwrap();

        Self {
            address: StorageAddressPaddedTarget(ArrayTarget(targets)),
            length,
        }
    }

    pub fn append(
        self,
        append: PartialStorageAddressTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let final_length = builder.add(self.length.to_target(), append.length.to_target());
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
            let appended_read_idx = builder.sub(self_read_idx, self.length.to_target());

            // `appended_read_idx` is valid iff `self_read_idx` >= `self.length`.
            // Elsewhere we don't care about actual idx and set it to zero, as we don't use it's value.
            let appended_read_idx_valid = list_le_circuit(
                builder,
                vec![self.length.to_target()],
                vec![self_read_idx],
                32,
            );
            // Returns `if b { x } else { y }`.
            let appended_read_idx =
                builder.select(appended_read_idx_valid, appended_read_idx, zero);

            let self_nibble = self.address.constant_read(i);
            let appended_nibble = append
                .address
                .random_read(appended_read_idx.into(), builder);

            // Check if `self.length` <= `i`
            let select_nibble_from_appended = list_le_circuit(
                builder,
                vec![self.length.to_target()],
                vec![self_read_idx],
                32,
            );

            // Returns `if b { x } else { y }`.
            builder.select(
                select_nibble_from_appended,
                appended_nibble.to_target(),
                self_nibble.to_target(),
            )
        });

        let address = StorageAddressPaddedTarget::parse_exact(&mut address_targets);

        Self {
            address,
            length: final_length.into(),
        }
    }
}

#[cfg(test)]
mod tests_common {
    use super::*;
    use plonky2::iop::witness::{PartialWitness, WitnessWrite};

    pub fn pad_byte_vec<const L: usize>(data: Vec<u8>) -> [u8; L] {
        assert!(data.len() <= L);
        data.into_iter()
            .chain(iter::repeat(0))
            .take(L)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    pub fn create_address_target(
        nibbles: &[u8],
        builder: &mut CircuitBuilder<F, D>,
        witness: &mut PartialWitness<F>,
    ) -> PartialStorageAddressTarget {
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

        PartialStorageAddressTarget {
            length: length.into(),
            address: StorageAddressPaddedTarget::parse_exact(&mut nibble_targets),
        }
    }
}

#[cfg(test)]
mod address_tests {
    use self::tests_common::pad_byte_vec;

    use super::{tests_common::create_address_target, *};
    use plonky2::{
        iop::witness::PartialWitness,
        plonk::{
            circuit_data::{CircuitConfig, CircuitData},
            proof::ProofWithPublicInputs,
        },
    };

    #[test]
    fn test_address_append() {
        address_append_test_case(&[], &[], Some(&[]));

        address_append_test_case(&[0xA], &[0x3], Some(&[0xA, 0x3]));

        address_append_test_case(&[0x1, 0x2, 0x3, 0x4], &[], Some(&[0x1, 0x2, 0x3, 0x4]));

        address_append_test_case(
            &vec![0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            Some(&vec![0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES]),
        );
    }

    #[test]
    #[should_panic(
        expected = "assertion `left == right` failed: Partition containing Wire(Wire { row: 130, column: 33 }) was set twice with different values: 0 != 1"
    )]
    fn test_address_append_overflow_fails() {
        address_append_test_case(
            &vec![0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES],
            &[0x1],
            None,
        );
    }

    #[test]
    fn test_address_append_have_constant_verifier_data() {
        let (first_cd, _) = build_test_case_circuit(&[0xA], &[0x3], Some(&[0xA, 0x3]));
        let (second_cd, _) = build_test_case_circuit(
            &vec![0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            &[0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES / 2],
            Some(&vec![0xA; MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES]),
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
        address_from_half_byte_targets_safe_test_case(&pad_byte_vec(vec![]), 0, &vec![]);

        address_from_half_byte_targets_safe_test_case(
            &pad_byte_vec(vec![1, 2, 3, 4]),
            4,
            &vec![1, 2, 3, 4],
        );

        address_from_half_byte_targets_safe_test_case(
            &pad_byte_vec(vec![1, 2, 3, 4, 5]),
            3,
            &vec![1, 2, 3],
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
            .into_iter()
            .map(|byte| HalfByteTarget::constant(*byte, &mut builder))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let length = builder.constant(F::from_canonical_usize(length));

        let address = PartialStorageAddressTarget::from_half_byte_targets_safe(
            targets,
            length.into(),
            &mut builder,
        );

        let expected_address = create_address_target(expected_data, &mut builder, &mut pw);

        address.register_as_public_inputs(&mut builder);

        address.connect(&expected_address, &mut builder);

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}

#[cfg(test)]
mod node_data_padded_tests {
    use plonky2::plonk::circuit_data::CircuitConfig;

    use self::tests_common::pad_byte_vec;

    use super::*;

    #[test]
    fn test_node_data_padded_random_read() {
        test_case(
            &[
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
            ],
            0,
            0,
        );

        let meaningful_data = &[
            pad_byte_vec(vec![0xAA; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xBB; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xCC; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xDD; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xEE; NODE_DATA_BLOCK_BYTES]),
        ];

        test_case(meaningful_data, 0, 0xAA);
        test_case(meaningful_data, NODE_DATA_BLOCK_BYTES - 1, 0xAA);
        test_case(meaningful_data, NODE_DATA_BLOCK_BYTES, 0xBB);
        test_case(
            meaningful_data,
            NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS - 1,
            0xEE,
        );
    }

    fn test_case(
        data: &[[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
        at: usize,
        expected_data: u8,
    ) {
        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let node_data = BranchNodeDataPaddedTarget::add_virtual_unsafe(&mut builder);
        node_data.set_witness(data, &mut pw);

        let at_target = builder.add_virtual_target();
        pw.set_target(at_target, F::from_canonical_usize(at));

        let data = node_data.random_read(at_target.into(), &mut builder);
        let expected = ByteTarget::constant(expected_data, &mut builder);

        data.connect(&expected, &mut builder);

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}
