use super::{
    super::{
        BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
    },
    scale_compact_integer_parser::{self, ScaleCompactIntegerParserInputTarget},
};
use crate::{
    common::{
        array_to_bits,
        targets::{
            impl_target_set, ArrayTarget, Blake2Target, HalfByteTarget, SingleTarget, TargetSet,
        },
    },
    consts::{BLAKE2_DIGEST_SIZE, BLAKE2_DIGEST_SIZE_IN_BITS},
    prelude::*,
    ProofWithCircuitData,
};
use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;
use plonky2_field::types::Field;

impl_target_set! {
    pub struct ChildNodeParserTarget {
        pub node_data: BranchNodeDataPaddedTarget,

        pub read_offset: SingleTarget,
        pub resulting_read_offset: SingleTarget,

        pub assert_child_hash: BoolTarget,
        pub claimed_child_hash: Blake2Target,
    }
}

struct ChildNodeParser {
    pub node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],

    pub read_offset: usize,

    pub assert_child_hash: bool,
    pub claimed_child_hash: [bool; BLAKE2_DIGEST_SIZE_IN_BITS],
}

impl ChildNodeParser {
    fn prove(self) -> ProofWithCircuitData<ChildNodeParserTarget> {
        log::info!("Proving child node parser...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let node_data = BranchNodeDataPaddedTarget::add_virtual_unsafe(&mut builder);
        node_data.set_witness(&self.node_data, &mut pw);

        let read_offset = builder.add_virtual_target();
        pw.set_target(read_offset, F::from_canonical_usize(self.read_offset));

        let assert_child_hash = builder.add_virtual_bool_target_unsafe();
        pw.set_bool_target(assert_child_hash, self.assert_child_hash);

        let claimed_child_hash = Blake2Target::add_virtual_unsafe(&mut builder);
        claimed_child_hash.set_witness(&self.claimed_child_hash, &mut pw);

        // Read only one byte as we don't support compact integers in other modes than single-byte.
        let encoded_length_size = builder.one();
        let encoded_length = node_data.random_read(read_offset.into(), &mut builder);
        let encoded_child_data_length = scale_compact_integer_parser::define(
            ScaleCompactIntegerParserInputTarget {
                first_byte: encoded_length,
            },
            &mut builder,
        )
        .decoded
        .to_target();

        let read_data_at = builder.add(read_offset, encoded_length_size);
        let potential_child_hash_data: ArrayTarget<_, BLAKE2_DIGEST_SIZE> =
            node_data.random_read_array(read_data_at.into(), &mut builder);
        let mut potential_child_hash_data_bits = potential_child_hash_data
            .0
            .into_iter()
            .map(|byte| {
                byte.to_bit_targets(&mut builder)
                    .0
                    .into_iter()
                    .map(|target| target.target)
                    .rev()
            })
            .flatten();
        let potential_child_hash = Blake2Target::parse_exact(&mut potential_child_hash_data_bits);
        let child_hash_matches =
            claimed_child_hash.check_equal(&potential_child_hash, &mut builder);
        let child_hash_not_matches = builder.not(child_hash_matches);

        let invalid_child_hash = builder.and(child_hash_not_matches, assert_child_hash);
        builder.assert_zero(invalid_child_hash.target);

        let resulting_read_offset = builder.add_many(vec![
            read_offset,
            encoded_length_size,
            encoded_child_data_length,
        ]);

        ChildNodeParserTarget {
            node_data,
            read_offset: read_offset.into(),
            resulting_read_offset: resulting_read_offset.into(),
            assert_child_hash,
            claimed_child_hash,
        }
        .register_as_public_inputs(&mut builder);

        let data = ProofWithCircuitData::from_builder(builder, pw);

        log::info!("Proven child node parser");

        data
    }
}
