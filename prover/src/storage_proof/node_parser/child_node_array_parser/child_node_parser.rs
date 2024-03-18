use super::{
    super::{
        BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
    },
    scale_compact_integer_parser::{self, ScaleCompactIntegerParserInputTarget},
};
use crate::{
    common::targets::{
        impl_target_set, ArrayTarget, Blake2Target, ParsableTargetSet, SingleTarget, TargetSet,
    },
    consts::{BLAKE2_DIGEST_SIZE, BLAKE2_DIGEST_SIZE_IN_BITS},
    prelude::*,
    ProofWithCircuitData,
};
use plonky2::{
    iop::{
        target::BoolTarget,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
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

pub struct ChildNodeParserData {
    node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],

    read_offset: usize,
    resulting_read_offset: usize,

    assert_child_hash: bool,
    claimed_child_hash: [bool; BLAKE2_DIGEST_SIZE_IN_BITS],
}

impl ParsableTargetSet for ChildNodeParserTarget {
    type PublicInputsData = ChildNodeParserData;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        ChildNodeParserData {
            node_data: BranchNodeDataPaddedTarget::parse_public_inputs(public_inputs),
            read_offset: SingleTarget::parse_public_inputs(public_inputs) as usize,
            resulting_read_offset: SingleTarget::parse_public_inputs(public_inputs) as usize,
            assert_child_hash: BoolTarget::parse_public_inputs(public_inputs),
            claimed_child_hash:
                ArrayTarget::<BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS>::parse_public_inputs(
                    public_inputs,
                ),
        }
    }
}

pub struct ChildNodeParser {
    pub node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],

    pub read_offset: usize,

    pub assert_child_hash: bool,
    pub claimed_child_hash: [bool; BLAKE2_DIGEST_SIZE_IN_BITS],
}

impl ChildNodeParser {
    pub fn prove(self) -> ProofWithCircuitData<ChildNodeParserTarget> {
        log::info!("Proving child node parser...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{common::array_to_bits, storage_proof::node_parser::tests_common::pad_byte_vec};
    use parity_scale_codec::{Compact, Encode};

    #[test]
    fn test_child_node_parser() {
        test_case(encode_not_claimed_node(0).collect(), 0, 1, None);

        let child_hash = vec![120, 200, 3, 10]
            .into_iter()
            .chain(vec![0; 27].into_iter())
            .chain(vec![99].into_iter())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let claimed_node: Vec<_> = encode_claimed_node(&child_hash).collect();

        test_case(
            claimed_node.clone(),
            0,
            claimed_node.len(),
            Some(child_hash),
        );

        let short_not_claimed_node: Vec<_> = encode_not_claimed_node(5).collect();

        test_case(
            vec![short_not_claimed_node.clone(), claimed_node.clone()]
                .into_iter()
                .flatten()
                .collect(),
            short_not_claimed_node.len(),
            claimed_node.len(),
            Some(child_hash),
        );

        let not_claimed_node: Vec<_> = encode_not_claimed_node(32).collect();
        let mut longest_node_data = vec![];
        for _ in 0..15 {
            longest_node_data.append(&mut not_claimed_node.clone());
        }
        longest_node_data.append(&mut claimed_node.clone());
        test_case(
            longest_node_data,
            15 * not_claimed_node.len(),
            claimed_node.len(),
            Some(child_hash),
        );
    }

    fn test_case(
        node_data: Vec<u8>,
        read_offset: usize,
        expected_read_data_len: usize,
        child_hash: Option<[u8; BLAKE2_DIGEST_SIZE]>,
    ) {
        let node_data_padded: [u8; NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS] =
            pad_byte_vec(node_data);
        let node_data_blocks = node_data_padded
            .chunks(NODE_DATA_BLOCK_BYTES)
            .map(|chunk| chunk.try_into().unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let claimed_child_hash = child_hash
            .map(|data| array_to_bits(&data).try_into().unwrap())
            .unwrap_or_else(|| [false; BLAKE2_DIGEST_SIZE_IN_BITS]);

        let parser = ChildNodeParser {
            node_data: node_data_blocks,
            read_offset,
            assert_child_hash: child_hash.is_some(),
            claimed_child_hash,
        };

        let proof = parser.prove();

        let public_inputs =
            ChildNodeParserTarget::parse_public_inputs_exact(&mut proof.pis().into_iter());

        assert_eq!(
            public_inputs.resulting_read_offset - public_inputs.read_offset,
            expected_read_data_len
        );

        proof.verify();
    }

    fn encode_not_claimed_node(length: usize) -> impl Iterator<Item = u8> {
        let mut data = Compact::<u32>(length as u32).encode();
        data.append(&mut vec![0; length]);
        data.into_iter()
    }

    fn encode_claimed_node(hash: &[u8; BLAKE2_DIGEST_SIZE]) -> impl Iterator<Item = u8> {
        let mut data = Compact::<u32>(BLAKE2_DIGEST_SIZE as u32).encode();
        data.append(&mut hash.to_vec());
        data.into_iter()
    }
}
