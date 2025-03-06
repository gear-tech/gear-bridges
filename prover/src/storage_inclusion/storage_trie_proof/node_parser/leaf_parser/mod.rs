//! ### Circuit that's used to parse encoded leaf node.

use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
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
        targets::{Blake2Target, TargetSet},
        ProofWithCircuitData,
    },
    impl_parsable_target_set,
    prelude::*,
    storage_inclusion::storage_trie_proof::{
        node_parser::{
            header_parser::HeaderDescriptor,
            leaf_parser::{
                hashed_data_parser::HashedDataParserInputTarget,
                inlined_data_parser::InlinedDataParserInputTarget,
            },
        },
        storage_address::StorageAddressTarget,
    },
};

mod hashed_data_parser;
mod inlined_data_parser;

impl_parsable_target_set! {
    /// `LeafParser` public inputs.
    pub struct LeafParserTarget {
        /// Encoded node data, padded to max leaf node encoded length.
        pub padded_node_data: LeafNodeDataPaddedTarget,
        /// Actual encoded data length.
        pub node_data_length: Target,
        /// Expected blake2b hash of data stored in this leaf.
        pub storage_data_hash: Blake2Target,
        /// Address that was previously composed by parsing all the parent nodes.
        pub partial_address: StorageAddressTarget,
        /// Final address of storage item.
        pub final_address: StorageAddressTarget
    }
}

pub struct LeafParser {
    /// Encoded leaf node data.
    pub node_data: Vec<u8>,
    /// Previously composed address nibbles. Note that we store nibbles as `u8` but they're must be
    /// in range (0..=15).
    pub partial_address_nibbles: Vec<u8>,
}

enum LeafType {
    Leaf,
    HashedValueLeaf,
}

impl LeafParser {
    pub fn prove(self) -> ProofWithCircuitData<LeafParserTarget> {
        log::debug!("Proving leaf node parser...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let node_data_length_target = builder.add_virtual_target();
        witness.set_target(
            node_data_length_target,
            F::from_canonical_usize(self.node_data.len()),
        );

        let (leaf_type, header_descriptor) =
            if HeaderDescriptor::hashed_value_leaf().prefix_matches(&self.node_data) {
                (
                    LeafType::HashedValueLeaf,
                    HeaderDescriptor::hashed_value_leaf(),
                )
            } else if HeaderDescriptor::leaf().prefix_matches(&self.node_data) {
                (LeafType::Leaf, HeaderDescriptor::leaf())
            } else {
                unimplemented!("Unsupported leaf type")
            };

        let node_data_target = LeafNodeDataPaddedTarget::add_virtual_safe(&mut builder);
        node_data_target.set_witness(&pad_byte_vec(self.node_data), &mut witness);

        let partial_address_target = StorageAddressTarget::add_virtual_unsafe(&mut builder);
        partial_address_target.set_witness(&self.partial_address_nibbles, &mut witness);

        let parsed_header = {
            let first_bytes = node_data_target.constant_read_array(0);
            let input = HeaderParserInputTarget { first_bytes };
            header_parser::define(input, header_descriptor, &mut builder)
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

        let (resulting_offset, data_hash) = match leaf_type {
            LeafType::HashedValueLeaf => {
                let parsed_data = {
                    let input = HashedDataParserInputTarget {
                        node_data: node_data_target.clone(),
                        read_offset: parsed_nibbles.resulting_offset,
                    };
                    hashed_data_parser::define(input, &mut builder)
                };

                (parsed_data.resulting_offset, parsed_data.data_hash)
            }
            LeafType::Leaf => {
                let parsed_data = {
                    let input = InlinedDataParserInputTarget {
                        node_data: node_data_target.clone(),
                        read_offset: parsed_nibbles.resulting_offset,
                    };
                    inlined_data_parser::define(input, &mut builder)
                };

                (parsed_data.resulting_offset, parsed_data.data_hash)
            }
        };

        resulting_offset.connect(&node_data_length_target, &mut builder);

        LeafParserTarget {
            padded_node_data: node_data_target,
            node_data_length: node_data_length_target,
            storage_data_hash: data_hash,
            partial_address: partial_address_target,
            final_address: parsed_nibbles.partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Proven leaf node parser");

        result
    }
}

#[cfg(test)]
mod tests {
    use trie_db::{node::Value, NibbleSlice, NodeCodec, TrieLayout};

    use super::*;
    use crate::{
        common::{array_to_bits, targets::ParsableTargetSet},
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

        let pis =
            LeafParserTarget::parse_public_inputs_exact(&mut proof.public_inputs().into_iter());

        let nibble_count = nibbles.len();
        let expected_address_nibbles = (0..nibble_count)
            .map(|idx| nibbles.at(idx))
            .collect::<Vec<_>>();

        assert_eq!(
            pis.final_address.length,
            expected_address_nibbles.len() as u64
        );
        assert_eq!(
            pis.final_address.padded_address,
            pad_byte_vec(expected_address_nibbles)
        );

        assert_eq!(&pis.storage_data_hash, &array_to_bits(&data_hash)[..]);
    }
}
