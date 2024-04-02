use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        generic_blake2::GenericBlake2,
        targets::{impl_parsable_target_set, Blake2Target, TargetSet},
        BuilderExt,
    },
    prelude::*,
    ProofWithCircuitData,
};

use super::{
    node_parser::branch_parser::BranchParser, storage_address::PartialStorageAddressTarget,
};

impl_parsable_target_set! {
    pub struct HashedBranchParserTarget {
        pub node_hash: Blake2Target,
        pub child_node_hash: Blake2Target,

        pub partial_address: PartialStorageAddressTarget,
        pub resulting_partial_address: PartialStorageAddressTarget,
    }
}

pub struct HashedBranchParser {
    pub branch_parser: BranchParser,
}

impl HashedBranchParser {
    pub fn prove(self) -> ProofWithCircuitData<HashedBranchParserTarget> {
        let hasher_proof = GenericBlake2 {
            data: self.branch_parser.node_data.clone(),
        }
        .prove();
        let branch_parser_proof = self.branch_parser.prove();

        log::info!("Composing hasher proof and branch parser proof...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let hasher_target = builder.recursively_verify_constant_proof(hasher_proof, &mut witness);
        let branch_parser_target =
            builder.recursively_verify_constant_proof(branch_parser_proof, &mut witness);

        hasher_target
            .length
            .connect(&branch_parser_target.node_data_length, &mut builder);

        let mut branch_parser_node_data = branch_parser_target.padded_node_data.into_targets_iter();
        let mut hasher_node_data = hasher_target.data.into_targets_iter();
        loop {
            let branch_parser_byte = branch_parser_node_data.next();
            let hasher_byte = hasher_node_data.next();

            match (branch_parser_byte, hasher_byte) {
                (Some(a), Some(b)) => {
                    builder.connect(a, b);
                }
                (Some(_), None) => {
                    panic!("Generic blake2 hasher circuit have insifficient maximum data length");
                }
                _ => break,
            }
        }

        HashedBranchParserTarget {
            node_hash: hasher_target.hash,
            child_node_hash: branch_parser_target.child_node_hash,
            partial_address: branch_parser_target.partial_address,
            resulting_partial_address: branch_parser_target.resulting_partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::from_builder(builder, witness);

        log::info!("Composed hasher proof and branch parser proof");

        result
    }
}
