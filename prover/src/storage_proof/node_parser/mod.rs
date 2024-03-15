use std::iter;

use plonky2::{iop::target::Target, plonk::circuit_builder::CircuitBuilder};
use plonky2_field::types::Field;
use plonky2_u32::gadgets::multiple_comparison::list_le_circuit;

use crate::{
    common::targets::{
        impl_array_target_wrapper, impl_target_set, ByteTarget, HalfByteTarget, SingleTarget,
        TargetSet,
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

impl_array_target_wrapper!(NodeDataBlockTarget, ByteTarget, NODE_DATA_BLOCK_BYTES);

impl_array_target_wrapper!(
    BranchNodeDataPaddedTarget,
    NodeDataBlockTarget,
    MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS
);

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
    // Invariant: all the data after `current_length` is zeroed.
    pub struct PartialStorageAddressTarget {
        pub address: StorageAddressPaddedTarget,
        pub current_length: SingleTarget
    }
}

impl PartialStorageAddressTarget {
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self {
            address: StorageAddressPaddedTarget::empty(builder),
            current_length: builder.zero().into(),
        }
    }

    pub fn append(
        self,
        append: PartialStorageAddressTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let final_length = builder.add(
            self.current_length.to_target(),
            append.current_length.to_target(),
        );
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
            let appended_read_idx = builder.sub(self_read_idx, self.current_length.to_target());

            // `appended_read_idx` is valid iff `self_read_idx` >= `self.current_length`.
            // Elsewhere we don't care about actual idx and set it to zero, as we don't use it's value.
            let appended_read_idx_valid = list_le_circuit(
                builder,
                vec![self.current_length.to_target()],
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

            // Check if `self.current_length` <= `i`
            let select_nibble_from_appended = list_le_circuit(
                builder,
                vec![self.current_length.to_target()],
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
            current_length: final_length.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plonky2::{
        iop::witness::{PartialWitness, WitnessWrite},
        plonk::{
            circuit_data::{CircuitConfig, CircuitData},
            proof::ProofWithPublicInputs,
        },
    };
    use std::iter;

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

    fn create_address_target(
        nibbles: &[u8],
        builder: &mut CircuitBuilder<F, D>,
        witness: &mut PartialWitness<F>,
    ) -> PartialStorageAddressTarget {
        assert!(nibbles.len() <= MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);
        let current_length = builder.add_virtual_target();
        witness.set_target(current_length, F::from_canonical_usize(nibbles.len()));

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
            current_length: current_length.into(),
            address: StorageAddressPaddedTarget::parse_exact(&mut nibble_targets),
        }
    }
}
