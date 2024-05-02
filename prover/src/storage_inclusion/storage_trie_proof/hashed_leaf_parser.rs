//! Contains circuit definition that's used to prove correct parsing of leaf node.

use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        generic_blake2::GenericBlake2,
        targets::{impl_parsable_target_set, Blake2Target, TargetSet},
        BuilderExt, ProofWithCircuitData,
    },
    prelude::*,
};

use super::{node_parser::leaf_parser::LeafParser, storage_address::PartialStorageAddressTarget};

impl_parsable_target_set! {
    /// Public inputs for `HashedLeafParser`.
    pub struct HashedLeafParserTarget {
        pub node_hash: Blake2Target,
        pub storage_data_hash: Blake2Target,

        /// Address composed from all the nodes from root to this.
        pub partial_address: PartialStorageAddressTarget,
        /// Address of storage item.
        pub final_address: PartialStorageAddressTarget,
    }
}

pub struct HashedLeafParser {
    /// Inner non-hashed leaf parser.
    pub leaf_parser: LeafParser,
}

impl HashedLeafParser {
    pub fn prove(self) -> ProofWithCircuitData<HashedLeafParserTarget> {
        let hasher_proof = GenericBlake2 {
            data: self.leaf_parser.node_data.clone(),
        }
        .prove();
        let leaf_parser_proof = self.leaf_parser.prove();

        log::debug!("Composing hasher proof and leaf parser proof...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let hasher_target = builder.recursively_verify_constant_proof(&hasher_proof, &mut witness);
        let leaf_parser_target =
            builder.recursively_verify_constant_proof(&leaf_parser_proof, &mut witness);

        hasher_target
            .length
            .connect(&leaf_parser_target.node_data_length, &mut builder);

        let mut leaf_parser_node_data = leaf_parser_target.padded_node_data.into_targets_iter();
        let mut hasher_node_data = hasher_target.data.into_targets_iter();
        loop {
            let leaf_parser_byte = leaf_parser_node_data.next();
            let hasher_byte = hasher_node_data.next();

            match (leaf_parser_byte, hasher_byte) {
                (Some(a), Some(b)) => {
                    builder.connect(a, b);
                }
                (Some(_), None) => {
                    panic!("Generic blake2 hasher circuit have insifficient maximum data length");
                }
                _ => break,
            }
        }

        HashedLeafParserTarget {
            node_hash: hasher_target.hash,
            storage_data_hash: leaf_parser_target.storage_data_hash,
            partial_address: leaf_parser_target.partial_address,
            final_address: leaf_parser_target.final_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Composed hasher proof and leaf parser proof...");

        result
    }
}
