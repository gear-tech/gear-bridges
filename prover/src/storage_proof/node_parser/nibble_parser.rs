use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use super::{
    NodeDataBlockTarget, PartialStorageAddressTarget, MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES,
};
use crate::{
    common::{
        array_to_bits,
        targets::{
            impl_target_set, ArrayTarget, Blake2Target, HalfByteTarget, SingleTarget, TargetSet,
        },
    },
    prelude::*,
    storage_proof::node_parser::MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct NibbleParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: SingleTarget,
        pub nibble_count: SingleTarget,
        pub partial_address: PartialStorageAddressTarget
    }
}

impl_target_set! {
    pub struct NibbleParserOutputTarget {
        pub resulting_offset: SingleTarget,
        pub partial_address: PartialStorageAddressTarget
    }
}

pub fn define(
    input: NibbleParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> NibbleParserOutputTarget {
    let potential_address_bytes: ArrayTarget<_, MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES> = input
        .first_node_data_block
        .random_read_array(input.read_offset.clone(), builder);

    let zero = HalfByteTarget::constant(0, builder);
    let mut potential_address_nibbles = potential_address_bytes
        .0
        .into_iter()
        .map(|byte| {
            let (l, m) = byte.to_half_byte_targets(builder);
            [m, l]
        })
        .flatten()
        .chain(iter::once(zero))
        .collect::<Vec<_>>();

    // It can be droppped, so we process it separately.
    let first_nibble = potential_address_nibbles.remove(0);
    let remaining_nibbles = potential_address_nibbles.try_into().unwrap();

    let nibble_count_odd = builder.low_bits(input.nibble_count.to_target(), 1, 32)[0];

    // If nibble count is odd:
    //  we take `input.nibble_count` nibbles from `remaining_nibbles`
    // If nibble count is even:
    //  we take `first_nibble` and input.nibble_count - 1` nibbles from `remaining_nibbles`
    let take_first_nibble = builder.not(nibble_count_odd).target;

    let take_from_remaining_nibbles =
        builder.add(input.nibble_count.to_target(), nibble_count_odd.target);
    let one = builder.one();
    let take_from_remaining_nibbles = builder.sub(take_from_remaining_nibbles, one);

    let zero = HalfByteTarget::constant(0, builder);
    let first_nibble_padded = vec![first_nibble]
        .into_iter()
        .chain(iter::repeat(zero))
        .take(MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let first_nibble_address_part = PartialStorageAddressTarget::from_half_byte_targets_safe(
        first_nibble_padded,
        take_first_nibble.into(),
        builder,
    );

    let remaining_nibbles_address_part = PartialStorageAddressTarget::from_half_byte_targets_safe(
        remaining_nibbles,
        take_from_remaining_nibbles.into(),
        builder,
    );

    let partial_address = input
        .partial_address
        .append(first_nibble_address_part, builder)
        .append(remaining_nibbles_address_part, builder);

    // read_bytes = (nibble_count - nibble_count % 2) / 2 + nibble_count % 2
    let read_bytes = builder.sub(input.nibble_count.to_target(), nibble_count_odd.target);
    let two = builder.two();
    let read_bytes = builder.div(read_bytes, two);
    let read_bytes = builder.add(read_bytes, nibble_count_odd.target);
    let resulting_offset = builder
        .add(input.read_offset.to_target(), read_bytes)
        .into();

    NibbleParserOutputTarget {
        partial_address,
        resulting_offset,
    }
}

// TODO: Add tests
