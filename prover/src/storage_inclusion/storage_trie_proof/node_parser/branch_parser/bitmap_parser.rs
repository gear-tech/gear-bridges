use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2_field::types::Field;

use crate::{
    common::{
        targets::{impl_target_set, HalfByteTarget, SingleTarget},
        xor_targets,
    },
    prelude::*,
    storage_inclusion::storage_trie_proof::node_parser::NodeDataBlockTarget,
};

impl_target_set! {
    pub struct BitmapParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: SingleTarget,
        pub claimed_child_node_nibble: HalfByteTarget,
    }
}

impl_target_set! {
    pub struct BitmapParserOutputTarget {
        pub resulting_offset: SingleTarget,
        pub overall_children_amount: SingleTarget,
        pub child_index_in_array: SingleTarget
    }
}

pub fn define(
    input: BitmapParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> BitmapParserOutputTarget {
    let first_byte = input
        .first_node_data_block
        .random_read(input.read_offset, builder);
    let read_offset = builder.add_const(input.read_offset.to_target(), F::ONE);
    let second_byte = input
        .first_node_data_block
        .random_read(read_offset.into(), builder);

    let first_bits = first_byte.to_bit_targets(builder);
    let second_bits = second_byte.to_bit_targets(builder);
    let bits = first_bits.0.into_iter().chain(second_bits.0);

    let mut child_amount_before_claimed_child = builder.zero();
    let mut before_claimed_child = builder._true();
    let mut overall_child_amount = builder.zero();
    for (bit_no, bit) in bits.enumerate() {
        overall_child_amount = builder.add(overall_child_amount, bit.target);

        let bit_no = builder.constant(F::from_canonical_usize(bit_no));
        let child_is_claimed =
            builder.is_equal(bit_no, input.claimed_child_node_nibble.to_target());

        // Check that bit is set to 1 in claimed child.
        let inv_bit = builder.not(bit);
        let incorrect_state = builder.and(child_is_claimed, inv_bit);
        builder.assert_zero(incorrect_state.target);

        before_claimed_child = xor_targets(before_claimed_child, child_is_claimed, builder);
        let masked_bit_value = builder.and(before_claimed_child, bit);
        child_amount_before_claimed_child =
            builder.add(child_amount_before_claimed_child, masked_bit_value.target);
    }

    let resulting_offset = builder
        .add_const(input.read_offset.to_target(), F::TWO)
        .into();
    BitmapParserOutputTarget {
        resulting_offset,
        overall_children_amount: overall_child_amount.into(),
        child_index_in_array: child_amount_before_claimed_child.into(),
    }
}

#[cfg(test)]
mod tests {
    use plonky2::{iop::witness::PartialWitness, plonk::circuit_data::CircuitConfig};

    use super::*;
    use crate::storage_inclusion::storage_trie_proof::node_parser::pad_byte_vec;

    #[test]
    fn test_bitmap_parser() {
        test_case(
            [0b00_00_00_01, 0b00_00_00_00],
            0,
            Some(ExpectedData {
                overall_children_amount: 1,
                child_index: 0,
            }),
        );

        test_case(
            [0b00_00_00_00, 0b00_00_01_00],
            10,
            Some(ExpectedData {
                overall_children_amount: 1,
                child_index: 0,
            }),
        );

        test_case(
            [0b01_11_00_11, 0b01_10_01_11],
            10,
            Some(ExpectedData {
                overall_children_amount: 10,
                child_index: 7,
            }),
        );
    }

    #[test]
    #[should_panic(
        expected = "Partition containing Wire(Wire { row: 7, column: 59 }) was set twice with different values: 0 != 1"
    )]
    fn test_bitmap_parser_wrong_claimed_child_fails() {
        test_case([0b01_11_11_11, 0b11_11_11_11], 7, None);
    }

    struct ExpectedData {
        overall_children_amount: usize,
        child_index: usize,
    }

    fn test_case(bitmap: [u8; 2], claimed_nibble: u8, expected_data: Option<ExpectedData>) {
        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::<F, D>::new(config);
        let pw = PartialWitness::new();

        let data_block_target =
            NodeDataBlockTarget::constant(&pad_byte_vec(bitmap.to_vec()), &mut builder);
        let read_offset = builder.zero().into();
        let claimed_child_node_nibble = HalfByteTarget::constant(claimed_nibble, &mut builder);

        let input = BitmapParserInputTarget {
            first_node_data_block: data_block_target,
            read_offset,
            claimed_child_node_nibble,
        };

        let output = define(input, &mut builder);

        if let Some(ExpectedData {
            overall_children_amount,
            child_index,
        }) = expected_data
        {
            let overall_children_amount =
                builder.constant(F::from_canonical_usize(overall_children_amount));
            builder.connect(
                overall_children_amount,
                output.overall_children_amount.to_target(),
            );

            let child_index = builder.constant(F::from_canonical_usize(child_index));
            builder.connect(child_index, output.child_index_in_array.to_target());
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}
