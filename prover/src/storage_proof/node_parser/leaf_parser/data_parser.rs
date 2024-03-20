use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2_field::types::Field;

use crate::{
    common::targets::{impl_target_set, ArrayTarget, Blake2Target, SingleTarget, TargetSet},
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    storage_proof::node_parser::NodeDataBlockTarget,
};

impl_target_set! {
    pub struct DataParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: SingleTarget,
    }
}

impl_target_set! {
    pub struct DataParserOutputTarget {
        pub resulting_offset: SingleTarget,
        pub data_hash: Blake2Target
    }
}

pub fn define(
    input: DataParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> DataParserOutputTarget {
    let hash_data: ArrayTarget<_, BLAKE2_DIGEST_SIZE> = input
        .first_node_data_block
        .random_read_array(input.read_offset, builder);

    let mut hash_data_bits = hash_data.0.iter().flat_map(|byte_target| {
        byte_target
            .to_bit_targets(builder)
            .0
            .into_iter()
            .map(|target| target.target)
            .rev()
    });
    let data_hash = Blake2Target::parse_exact(&mut hash_data_bits);

    let resulting_offset = builder
        .add_const(
            input.read_offset.to_target(),
            F::from_canonical_usize(BLAKE2_DIGEST_SIZE),
        )
        .into();

    DataParserOutputTarget {
        resulting_offset,
        data_hash,
    }
}
