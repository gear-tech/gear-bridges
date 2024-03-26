use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_field::types::Field;

use super::{
    header_parser::{self, HeaderParserInputTarget},
    nibble_parser::{self, NibbleParserInputTarget},
    LeafNodeDataPaddedTarget,
};
use crate::{
    common::{
        pad_byte_vec,
        targets::{Blake2Target, SingleTarget, TargetSet},
    },
    impl_parsable_target_set,
    prelude::*,
    storage_proof::storage_trie_proof::{
        node_parser::{
            header_parser::HeaderDescriptor, leaf_parser::data_parser::DataParserInputTarget,
        },
        storage_address::PartialStorageAddressTarget,
    },
    ProofWithCircuitData,
};

mod data_parser;

impl_parsable_target_set! {
    pub struct LeafParserTarget {
        pub padded_node_data: LeafNodeDataPaddedTarget,
        pub node_data_length: SingleTarget,

        pub storage_data_hash: Blake2Target,

        pub partial_address: PartialStorageAddressTarget,
        pub final_address: PartialStorageAddressTarget
    }
}

pub struct LeafParser {
    pub node_data: Vec<u8>,
    pub partial_address_nibbles: Vec<u8>,
}

impl LeafParser {
    pub fn prove(self) -> ProofWithCircuitData<LeafParserTarget> {
        log::info!("Proving leaf node parser...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let node_data_length_target: SingleTarget = builder.add_virtual_target().into();
        witness.set_target(
            node_data_length_target.to_target(),
            F::from_canonical_usize(self.node_data.len()),
        );

        let node_data_target = LeafNodeDataPaddedTarget::add_virtual_safe(&mut builder);
        node_data_target.set_witness(&pad_byte_vec(self.node_data), &mut witness);

        let partial_address_target = PartialStorageAddressTarget::add_virtual_unsafe(&mut builder);
        partial_address_target.set_witness(&self.partial_address_nibbles, &mut witness);

        let parsed_header = {
            let first_bytes = node_data_target.constant_read_array(0);
            let input = HeaderParserInputTarget { first_bytes };
            header_parser::define(input, HeaderDescriptor::hashed_value_leaf(), &mut builder)
        };

        let parsed_nibbles = {
            let input = NibbleParserInputTarget {
                first_node_data_block: node_data_target.clone(),
                read_offset: parsed_header.resulting_offset,
                nibble_count: parsed_header.nibble_count,
                partial_address: partial_address_target.clone(),
            };
            nibble_parser::define(input, &mut builder)
        };

        let parsed_data = {
            let input = DataParserInputTarget {
                first_node_data_block: node_data_target.clone(),
                read_offset: parsed_nibbles.resulting_offset,
            };
            data_parser::define(input, &mut builder)
        };

        parsed_data
            .resulting_offset
            .connect(&node_data_length_target, &mut builder);

        LeafParserTarget {
            padded_node_data: node_data_target,
            node_data_length: node_data_length_target,
            storage_data_hash: parsed_data.data_hash,
            partial_address: partial_address_target,
            final_address: parsed_nibbles.partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::from_builder(builder, witness);

        log::info!("Proven leaf node parser");

        result
    }
}

#[cfg(test)]
mod tests {
    use trie_db::{node::Value, NibbleSlice, NodeCodec, TrieLayout};

    use super::*;
    use crate::{
        common::{array_to_bits, pad_byte_vec, targets::ParsableTargetSet},
        prelude::consts::BLAKE2_DIGEST_SIZE,
    };

    #[test]
    fn test_leaf_node_parser() {
        test_case(NibbleSlice::new(&[]), [0; BLAKE2_DIGEST_SIZE]);

        test_case(NibbleSlice::new(&[1, 2, 3, 4]), [0x0D; BLAKE2_DIGEST_SIZE]);

        test_case(
            NibbleSlice::new_offset(&[0x0A, 0xBB, 0xDF], 1),
            [0xA1; BLAKE2_DIGEST_SIZE],
        );
    }

    fn test_case(nibbles: NibbleSlice, data_hash: [u8; BLAKE2_DIGEST_SIZE]) {
        type TrieCodec = <sp_trie::LayoutV1<sp_core::Blake2Hasher> as TrieLayout>::Codec;

        let node_data =
            TrieCodec::leaf_node(nibbles.right_iter(), nibbles.len(), Value::Node(&data_hash));

        let proof = LeafParser {
            node_data,
            partial_address_nibbles: vec![],
        }
        .prove();

        assert!(proof.verify());

        let pis = LeafParserTarget::parse_public_inputs_exact(&mut proof.pis().into_iter());

        let nibble_count = nibbles.len();
        let expected_address_nibbles = (0..nibble_count)
            .map(|idx| nibbles.at(idx))
            .collect::<Vec<_>>();

        assert_eq!(
            pis.final_address.length,
            expected_address_nibbles.len() as u64
        );
        assert_eq!(
            pis.final_address.address,
            pad_byte_vec(expected_address_nibbles)
        );

        assert_eq!(&pis.storage_data_hash, &array_to_bits(&data_hash)[..]);
    }
}
