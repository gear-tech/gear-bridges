use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::targets::{ArrayTarget, Blake2Target, HalfByteTarget},
    consts::BLAKE2_DIGEST_SIZE,
    impl_target_set,
    storage_proof::storage_address::{
        PartialStorageAddressTarget, MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES,
    },
    ProofWithCircuitData,
};

use self::{
    bitmap_parser::BitmapParserInputTarget, branch_header_parser::BranchHeaderParserInputTarget,
};

use super::{
    compose_padded_node_data,
    nibble_parser::{self, NibbleParserInputTarget},
    BranchNodeDataPaddedTarget,
};

mod bitmap_parser;
mod branch_header_parser;
mod child_node_array_parser;

impl_target_set! {
    pub struct BranchParserTarget {
        pub node_data_hash: Blake2Target,
        pub child_node_hash: Blake2Target,

        pub partial_address: PartialStorageAddressTarget,
        pub resulting_partial_address: PartialStorageAddressTarget,
    }
}

pub struct BranchParser {
    pub node_data: Vec<u8>,

    pub children_data_offset: usize,
    pub children_lengths: Vec<usize>,

    pub claimed_child_node_nibble: u8,
    pub partial_address_nibbles: Vec<u8>,
}

impl BranchParser {
    pub fn prove(&self) -> ProofWithCircuitData<BranchParserTarget> {
        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let node_data_target = BranchNodeDataPaddedTarget::add_virtual_safe(&mut builder);
        node_data_target.set_witness(
            &compose_padded_node_data(self.node_data.clone()),
            &mut witness,
        );

        let partial_address_target = PartialStorageAddressTarget::add_virtual_unsafe(&mut builder);
        partial_address_target.set_witness(&self.partial_address_nibbles, &mut witness);

        let claimed_child_node_nibble_target = builder.add_virtual_target();
        let claimed_child_node_nibble_target =
            HalfByteTarget::from_target_safe(claimed_child_node_nibble_target, &mut builder);

        let first_node_data_block = node_data_target.constant_read(0);

        let parsed_node_header = {
            let first_byte = node_data_target.constant_read(0).constant_read(0);
            let second_byte = node_data_target.constant_read(0).constant_read(1);
            let first_bytes = ArrayTarget([first_byte, second_byte]);

            let input = BranchHeaderParserInputTarget { first_bytes };
            branch_header_parser::define(input, &mut builder)
        };

        let parsed_nibbles = {
            let input = NibbleParserInputTarget {
                first_node_data_block: first_node_data_block.clone(),
                read_offset: parsed_node_header.resulting_offset,
                nibble_count: parsed_node_header.nibble_count,
                partial_address: partial_address_target,
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

        // TODO

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
