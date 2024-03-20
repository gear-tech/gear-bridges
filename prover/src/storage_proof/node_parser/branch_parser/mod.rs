use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_field::types::Field;
use plonky2_sha256::circuit::array_to_bits;

use crate::{
    common::{
        targets::{ArrayTarget, Blake2Target, HalfByteTarget, SingleTarget, TargetSet},
        ConstantRecursiveVerifier,
    },
    consts::BLAKE2_DIGEST_SIZE,
    impl_target_set,
    prelude::*,
    storage_proof::{
        node_parser::branch_parser::child_node_array_parser::ChildNodeArrayParserTarget,
        storage_address::{PartialStorageAddressTarget, MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES},
    },
    ProofWithCircuitData,
};

use self::{
    bitmap_parser::BitmapParserInputTarget, branch_header_parser::BranchHeaderParserInputTarget,
    child_node_array_parser::ChildNodeArrayParser,
};

use super::{
    compose_padded_node_data,
    nibble_parser::{self, NibbleParserInputTarget},
    BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
};

mod bitmap_parser;
mod branch_header_parser;
mod child_node_array_parser;

impl_target_set! {
    pub struct BranchParserTarget {
        pub padded_node_data: BranchNodeDataPaddedTarget,
        pub node_data_length: SingleTarget,

        pub child_node_hash: Blake2Target,

        pub partial_address: PartialStorageAddressTarget,
        pub resulting_partial_address: PartialStorageAddressTarget,
    }
}

// TODO: Decode it from node_data using TrieCodec, providing:
// node_data: Vec<u8>
// claimed_child_node_nibble: u8
// partial_address_nibbles: Vec<u8>
pub struct BranchParser {
    pub padded_node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
    pub node_data_length: usize,

    pub children_data_offset: usize,
    pub children_lengths: Vec<usize>,

    pub claimed_child_node_nibble: u8,
    pub claimed_child_index_in_array: usize,
    pub claimed_child_hash: [u8; BLAKE2_DIGEST_SIZE],

    pub partial_address_nibbles: Vec<u8>,
}

impl BranchParser {
    pub fn prove(&self) -> ProofWithCircuitData<BranchParserTarget> {
        let child_node_parser_proof = ChildNodeArrayParser {
            initial_data: child_node_array_parser::InitialData {
                node_data: self.padded_node_data.clone(),
                read_offset: self.children_data_offset,
                claimed_child_index_in_array: self.claimed_child_index_in_array,
                claimed_child_hash: self.claimed_child_hash,
            },
            children_lengths: self.children_lengths.clone(),
        }
        .prove();

        log::info!("Proving branch node parser...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let node_data_target = BranchNodeDataPaddedTarget::add_virtual_safe(&mut builder);

        let partial_address_target = PartialStorageAddressTarget::add_virtual_unsafe(&mut builder);

        let node_data_length_target: SingleTarget = builder.add_virtual_target().into();

        let claimed_child_node_nibble_target = builder.add_virtual_target();
        let claimed_child_node_nibble_target =
            HalfByteTarget::from_target_safe(claimed_child_node_nibble_target, &mut builder);

        let child_node_hash_target = Blake2Target::add_virtual_safe(&mut builder);

        let first_node_data_block = node_data_target.constant_read(0);

        let parsed_node_header = {
            let first_byte = first_node_data_block.constant_read(0);
            let second_byte = first_node_data_block.constant_read(1);
            let first_bytes = ArrayTarget([first_byte, second_byte]);

            let input = BranchHeaderParserInputTarget { first_bytes };
            branch_header_parser::define(input, &mut builder)
        };

        let parsed_nibbles = {
            let input = NibbleParserInputTarget {
                first_node_data_block: first_node_data_block.clone(),
                read_offset: parsed_node_header.resulting_offset,
                nibble_count: parsed_node_header.nibble_count,
                partial_address: partial_address_target.clone(),
            };
            nibble_parser::define(input, &mut builder)
        };

        let child_nibble_address_part = PartialStorageAddressTarget::from_single_nibble_target(
            claimed_child_node_nibble_target,
            &mut builder,
        );
        let resulting_address = parsed_nibbles
            .partial_address
            .append(child_nibble_address_part, &mut builder);

        let parsed_bitmap = {
            let input = BitmapParserInputTarget {
                first_node_data_block,
                read_offset: parsed_nibbles.resulting_offset,
                claimed_child_node_nibble: claimed_child_node_nibble_target,
            };

            bitmap_parser::define(input, &mut builder)
        };

        {
            let ChildNodeArrayParserTarget {
                node_data,
                initial_read_offset,
                final_read_offset,
                overall_children_amount,
                claimed_child_index_in_array,
                claimed_child_hash,
            } = builder.recursively_verify_constant_proof(child_node_parser_proof, &mut witness);

            node_data.connect(&node_data_target, &mut builder);
            initial_read_offset.connect(&parsed_bitmap.resulting_offset, &mut builder);
            final_read_offset.connect(&node_data_length_target, &mut builder);
            overall_children_amount.connect(&parsed_bitmap.overall_children_amount, &mut builder);
            claimed_child_index_in_array.connect(&parsed_bitmap.child_index_in_array, &mut builder);
            claimed_child_hash.connect(&child_node_hash_target, &mut builder);
        }

        BranchParserTarget {
            padded_node_data: node_data_target,
            node_data_length: node_data_length_target,
            child_node_hash: child_node_hash_target,
            partial_address: partial_address_target,
            resulting_partial_address: resulting_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::from_builder(builder, witness);

        log::info!("Proven branch node parser...");

        result
    }
}
