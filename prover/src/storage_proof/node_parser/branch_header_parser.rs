use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2_field::types::Field;

use super::NodeDataBlockTarget;
use crate::{
    common::targets::{impl_target_set, SingleTarget, TargetSet},
    prelude::*,
};

impl_target_set! {
    pub struct BranchHeaderParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
    }
}

impl_target_set! {
    pub struct BranchHeaderParserOutputTarget {
        pub nibble_count: SingleTarget,
        pub resulting_offset: SingleTarget,
    }
}

pub fn define(
    input: BranchHeaderParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> BranchHeaderParserOutputTarget {
    let first_byte = input.first_node_data_block.constant_read(0);
    let second_byte = input.first_node_data_block.constant_read(1);

    let first_byte_bits = first_byte.into_bits(builder);
    let node_prefix = (
        first_byte_bits.constant_read(7),
        first_byte_bits.constant_read(6),
    );

    let false_target = builder._false();
    let true_target = builder._true();
    // Should be 0b10
    builder.connect(node_prefix.0.target, true_target.target);
    builder.connect(node_prefix.1.target, false_target.target);

    let mask_value = builder.constant(F::from_canonical_u8(0b10_00_00_00));
    let first_byte_value = builder.sub(first_byte.to_target(), mask_value);
    let second_byte_value = second_byte.to_target();

    let max_first_byte_value = builder.constant(F::from_canonical_u8(0b00_11_11_11));
    let max_second_byte_value = builder.constant(F::from_canonical_u8(0b11_11_11_11));

    let first_val_is_max = builder.is_equal(first_byte_value, max_first_byte_value);
    let second_val_is_max = builder.is_equal(second_byte_value, max_second_byte_value);

    // First case:
    // 0b10[6-bit value < 63]
    let first_case_appliable = builder.not(first_val_is_max);

    // Second case:
    // 0b10111111 0b[8-bit value < 255]
    let second_case_appliable = {
        let second_val_not_max = builder.not(second_val_is_max);
        builder.and(first_val_is_max, second_val_not_max)
    };

    // There are more cases, like 0b11111111 0b11111111 0b[8-bit value < 255]
    // but we don't process them.

    let any_case_appliable = builder.or(first_case_appliable, second_case_appliable);
    builder.connect(any_case_appliable.target, true_target.target);
    // after this check we can be sure that first_case_appliable = not(second_case_appliable)

    let first_case_value = first_byte_value;
    let second_case_value = builder.add(second_byte_value, max_first_byte_value);

    let one_target = builder.one();
    let two_target = builder.two();
    // if b { x } else { y } corresponding to docs.
    let nibble_count = builder.select(first_case_appliable, first_case_value, second_case_value);
    let resulting_offset = builder.select(first_case_appliable, one_target, two_target);

    BranchHeaderParserOutputTarget {
        nibble_count: nibble_count.into(),
        resulting_offset: resulting_offset.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{super::NODE_DATA_BLOCK_BYTES, *};
    use plonky2::{iop::witness::PartialWitness, plonk::circuit_data::CircuitConfig};
    use std::iter;

    #[test]
    fn test_branch_header_parser() {
        test_case(
            &pad(vec![0b10_01_00_00]),
            Some(ExpectedData {
                nibble_count: 16,
                resulting_offset: 1,
            }),
        );

        test_case(
            &pad(vec![0b10_11_11_11, 0b_00_00_00_00]),
            Some(ExpectedData {
                nibble_count: 63,
                resulting_offset: 2,
            }),
        );

        test_case(
            &pad(vec![0b10_11_11_11, 0b_10_00_00_00]),
            Some(ExpectedData {
                nibble_count: 63 + 128,
                resulting_offset: 2,
            }),
        );
    }

    #[test]
    #[should_panic(
        expected = "Partition containing Wire(Wire { row: 4, column: 3 }) was set twice with different values: 1 != 0"
    )]
    fn test_branch_header_parser_value_overflow_panics() {
        test_case(&pad(vec![0b10_11_11_11, 0b11_11_11_11]), None);
    }

    #[test]
    #[should_panic(
        expected = "Partition containing Wire(Wire { row: 0, column: 8 }) was set twice with different values: 1 != 0"
    )]
    fn test_branch_header_parser_wrong_prefix_panics() {
        test_case(&pad(vec![0b00_00_00_11]), None);
    }

    struct ExpectedData {
        nibble_count: u64,
        resulting_offset: u64,
    }

    fn pad(data: Vec<u8>) -> [u8; NODE_DATA_BLOCK_BYTES] {
        data.into_iter()
            .chain(iter::repeat(0))
            .take(NODE_DATA_BLOCK_BYTES)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    fn test_case(node_data: &[u8; NODE_DATA_BLOCK_BYTES], expected_data: Option<ExpectedData>) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let pw = PartialWitness::new();

        let mut node_data_targets = node_data
            .iter()
            .map(|byte| builder.constant(F::from_canonical_u8(*byte)));
        let node_data_block = NodeDataBlockTarget::parse_exact(&mut node_data_targets);

        let input_target = BranchHeaderParserInputTarget {
            first_node_data_block: node_data_block,
        };

        let output_target = define(input_target, &mut builder);

        if let Some(ExpectedData {
            nibble_count,
            resulting_offset,
        }) = expected_data
        {
            let nibble_count = builder.constant(F::from_canonical_u64(nibble_count));
            let resulting_offset = builder.constant(F::from_canonical_u64(resulting_offset));

            builder.connect(nibble_count, output_target.nibble_count.to_target());
            builder.connect(resulting_offset, output_target.resulting_offset.to_target());
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw);

        let proof = proof.expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}
