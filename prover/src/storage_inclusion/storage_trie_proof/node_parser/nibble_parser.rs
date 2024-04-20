use plonky2::{iop::target::Target, plonk::circuit_builder::CircuitBuilder};
use std::iter;

use super::{NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::targets::{impl_target_set, ArrayTarget, HalfByteTarget},
    prelude::*,
    storage_inclusion::storage_trie_proof::storage_address::{
        MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES, MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES,
    },
};

impl_target_set! {
    pub struct NibbleParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: Target,
        pub nibble_count: Target,
        pub partial_address: PartialStorageAddressTarget
    }
}

impl_target_set! {
    pub struct NibbleParserOutputTarget {
        pub resulting_offset: Target,
        pub partial_address: PartialStorageAddressTarget
    }
}

pub fn define(
    input: NibbleParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> NibbleParserOutputTarget {
    let potential_address_bytes: ArrayTarget<_, MAX_STORAGE_ADDRESS_LENGTH_IN_BYTES> = input
        .first_node_data_block
        .random_read_array(input.read_offset, builder);

    let zero = HalfByteTarget::constant(0, builder);
    let mut potential_address_nibbles = potential_address_bytes
        .0
        .into_iter()
        .flat_map(|byte| {
            let (l, m) = byte.to_half_byte_targets(builder);
            [m, l]
        })
        .chain(iter::once(zero))
        .collect::<Vec<_>>();

    // It can be droppped, so we process it separately.
    let first_nibble = potential_address_nibbles.remove(0);
    let remaining_nibbles = potential_address_nibbles
        .try_into()
        .expect("Correct amount of nibbles");

    let nibble_count_odd = builder.low_bits(input.nibble_count, 1, 32)[0];

    // If nibble count is odd:
    //  we take `input.nibble_count` nibbles from `remaining_nibbles`
    // If nibble count is 0:
    //  we take `input.nibble_count` nibbles from `remaining_nibbles`
    // If nibble count is even:
    //  we take `first_nibble` and input.nibble_count - 1` nibbles from `remaining_nibbles`
    let zero = builder.zero();
    let nibble_count_is_zero = builder.is_equal(input.nibble_count, zero);
    let dont_take_first_nibble = builder.or(nibble_count_odd, nibble_count_is_zero);
    let take_first_nibble = builder.not(dont_take_first_nibble);

    let zero = HalfByteTarget::constant(0, builder);
    let first_nibble_padded = iter::once(first_nibble)
        .chain(iter::repeat(zero))
        .take(MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES)
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct amount of nibbles");
    let first_nibble_address_part = PartialStorageAddressTarget::from_half_byte_targets_safe(
        first_nibble_padded,
        take_first_nibble.target,
        builder,
    );

    let take_from_remaining = builder.sub(input.nibble_count, take_first_nibble.target);
    let remaining_nibbles_address_part = PartialStorageAddressTarget::from_half_byte_targets_safe(
        remaining_nibbles,
        take_from_remaining,
        builder,
    );

    let partial_address = input
        .partial_address
        .append(first_nibble_address_part, builder)
        .append(remaining_nibbles_address_part, builder);

    // read_bytes = (nibble_count - nibble_count % 2) / 2 + nibble_count % 2
    let read_bytes = builder.sub(input.nibble_count, nibble_count_odd.target);
    let two = builder.two();
    let read_bytes = builder.div(read_bytes, two);
    let read_bytes = builder.add(read_bytes, nibble_count_odd.target);
    let resulting_offset = builder.add(input.read_offset, read_bytes);

    NibbleParserOutputTarget {
        partial_address,
        resulting_offset,
    }
}

#[cfg(test)]
mod tests {
    use plonky2::{
        iop::witness::{PartialWitness, WitnessWrite},
        plonk::{
            circuit_builder::CircuitBuilder,
            circuit_data::{CircuitConfig, CircuitData},
            proof::ProofWithPublicInputs,
        },
    };
    use plonky2_field::types::Field;

    use super::*;
    use crate::{
        common::targets::TargetSet,
        storage_inclusion::storage_trie_proof::{
            node_parser::{pad_byte_vec, NodeDataBlockTarget, NODE_DATA_BLOCK_BYTES},
            storage_address::tests_common::create_address_target,
        },
    };

    #[test]
    fn test_nibble_parser() {
        test_case(
            pad_byte_vec(vec![]),
            0,
            Some(ExpectedData {
                resulting_offset: 0,
                address: vec![],
            }),
        );

        test_case(
            pad_byte_vec(vec![0xAA, 0xBB, 0xCC]),
            6,
            Some(ExpectedData {
                resulting_offset: 3,
                address: vec![0xA, 0xA, 0xB, 0xB, 0xC, 0xC],
            }),
        );

        test_case(
            pad_byte_vec(vec![0x0A, 0xBB, 0xCC]),
            5,
            Some(ExpectedData {
                resulting_offset: 3,
                address: vec![0xA, 0xB, 0xB, 0xC, 0xC],
            }),
        );
    }

    #[test]
    fn test_nibble_parser_have_constant_verifier_data() {
        let (_, first_cd) = build_test_case_circuit(
            pad_byte_vec(vec![]),
            0,
            Some(ExpectedData {
                resulting_offset: 0,
                address: vec![],
            }),
        );
        let (_, second_cd) = build_test_case_circuit(
            pad_byte_vec(vec![0x0A, 0xBB, 0xCC]),
            5,
            Some(ExpectedData {
                resulting_offset: 3,
                address: vec![0xA, 0xB, 0xB, 0xC, 0xC],
            }),
        );

        assert_eq!(first_cd.verifier_data(), second_cd.verifier_data());
    }

    struct ExpectedData {
        resulting_offset: usize,
        address: Vec<u8>,
    }

    fn test_case(
        data_block: [u8; NODE_DATA_BLOCK_BYTES],
        nibble_count: usize,
        expected_data: Option<ExpectedData>,
    ) {
        let (proof, circuit) = build_test_case_circuit(data_block, nibble_count, expected_data);
        circuit.verify(proof).expect("Failed to verify");
    }

    fn build_test_case_circuit(
        data_block: [u8; NODE_DATA_BLOCK_BYTES],
        nibble_count: usize,
        expected_data: Option<ExpectedData>,
    ) -> (ProofWithPublicInputs<F, C, D>, CircuitData<F, C, D>) {
        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let mut data_block_targets = data_block.into_iter().map(|byte| {
            let target = builder.add_virtual_target();
            pw.set_target(target, F::from_canonical_u8(byte));
            target
        });
        let data_block_target = NodeDataBlockTarget::parse_exact(&mut data_block_targets);

        let nibble_count_target = builder.add_virtual_target();
        pw.set_target(nibble_count_target, F::from_canonical_usize(nibble_count));

        let input = NibbleParserInputTarget {
            first_node_data_block: data_block_target,
            read_offset: builder.zero().into(),
            nibble_count: nibble_count_target.into(),
            partial_address: PartialStorageAddressTarget::empty(&mut builder),
        };

        let output = define(input, &mut builder);

        if let Some(ExpectedData {
            resulting_offset,
            address,
        }) = expected_data
        {
            let resulting_offset_target = builder.add_virtual_target();
            pw.set_target(
                resulting_offset_target,
                F::from_canonical_usize(resulting_offset),
            );
            builder.connect(resulting_offset_target, output.resulting_offset);

            let address = create_address_target(&address, &mut builder, &mut pw);
            address.connect(&output.partial_address, &mut builder);
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");

        (proof, circuit)
    }
}
