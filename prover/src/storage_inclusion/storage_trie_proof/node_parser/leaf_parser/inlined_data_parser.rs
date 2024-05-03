//! ### Circuit that's used to parse storage data from `Leaf` node.
//!
//! Currently supports only leafs with 32 bytes of inlined data.

use plonky2::{iop::target::Target, plonk::circuit_builder::CircuitBuilder};
use plonky2_field::types::Field;

use crate::{
    common::targets::{impl_target_set, ArrayTarget, Blake2Target, TargetSet},
    prelude::*,
    storage_inclusion::{
        scale_compact_integer_parser::single_byte::{
            define as define_single_byte_int_parser, InputTarget as SingleByteIntParserInput,
        },
        storage_trie_proof::node_parser::NodeDataBlockTarget,
    },
};

const INLINED_DATA_LENGTH: usize = 32;

impl_target_set! {
    pub struct InlindedDataParserInputTarget {
        // TODO: replace to `LeafNodeData`
        /// Node encoded data.
        pub first_node_data_block: NodeDataBlockTarget,
        /// From which offset to read stored data.
        pub read_offset: Target,
    }
}

impl_target_set! {
    pub struct InlinedDataParserOutputTarget {
        /// Offset of remaining node data.
        pub resulting_offset: Target,
        /// Blake2 hash of stored data.
        pub data_hash: Blake2Target
    }
}

pub fn define(
    input: InlindedDataParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> InlinedDataParserOutputTarget {
    log::debug!("    Composing inlined data parser");

    let first_byte = input
        .first_node_data_block
        .random_read(input.read_offset, builder);
    let parsed_length =
        define_single_byte_int_parser(SingleByteIntParserInput { first_byte }, builder);

    let desired_length = builder.constant(F::from_canonical_usize(INLINED_DATA_LENGTH));
    builder.connect(parsed_length.decoded, desired_length);

    let data_offset = builder.add_const(input.read_offset, F::ONE);

    let inlined_data: ArrayTarget<_, INLINED_DATA_LENGTH> = input
        .first_node_data_block
        .random_read_array(data_offset, builder);

    let inlined_data_bits = inlined_data
        .0
        .iter()
        .flat_map(|byte_target| byte_target.to_bit_targets(builder).0.into_iter().rev())
        .collect::<Vec<_>>();

    let mut inlined_data_hash =
        plonky2_blake2b256::circuit::blake2_circuit_from_targets(builder, inlined_data_bits)
            .into_iter()
            .map(|t| t.target);

    let data_hash = Blake2Target::parse_exact(&mut inlined_data_hash);

    let resulting_offset =
        builder.add_const(data_offset, F::from_canonical_usize(INLINED_DATA_LENGTH));

    InlinedDataParserOutputTarget {
        resulting_offset,
        data_hash,
    }
}
