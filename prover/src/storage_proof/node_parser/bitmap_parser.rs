use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;
use plonky2_field::{packed::PackedField, types::Field};

use super::{NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Blake2Target, HalfByteTarget, SingleTarget, TargetSet},
        xor_targets,
    },
    prelude::*,
    ProofWithCircuitData,
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
    let bits = first_bits
        .0
        .into_iter()
        .rev()
        .chain(second_bits.0.into_iter().rev());

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

// TODO: Add tests.
