use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{impl_target_set, Blake2Target, ParsableTargetSet, TargetSet},
        BuilderExt,
    },
    ProofWithCircuitData,
};

use super::BranchNodeData;

use self::{
    branch_node_chain::BranchNodeChainParserTarget, hashed_leaf_parser::HashedLeafParser,
    node_parser::leaf_parser::LeafParser, storage_address::PartialStorageAddressTarget,
};

mod branch_node_chain;
mod hashed_branch_parser;
mod hashed_leaf_parser;
mod node_parser;
pub mod storage_address;

use branch_node_chain::BranchNodeChain;

impl_target_set! {
    pub struct StorageTrieProofTarget {
        pub root_hash: Blake2Target,
        pub data_hash: Blake2Target,
        pub address: PartialStorageAddressTarget
    }
}

pub struct StorageTrieProof {
    pub branch_nodes: Vec<BranchNodeData>,
    pub leaf_node_data: Vec<u8>,
}

impl StorageTrieProof {
    pub fn prove(self) -> ProofWithCircuitData<StorageTrieProofTarget> {
        let branch_node_chain_proof = BranchNodeChain {
            nodes: self.branch_nodes,
        }
        .prove();

        let partial_address_nibbles = {
            let branch_node_chain_pis = BranchNodeChainParserTarget::parse_public_inputs_exact(
                &mut branch_node_chain_proof.public_inputs().into_iter(),
            );
            let partial_address = branch_node_chain_pis.partial_address;
            partial_address.padded_address[..partial_address.length as usize].to_vec()
        };

        let hashed_leaf_parser_proof = HashedLeafParser {
            leaf_parser: LeafParser {
                node_data: self.leaf_node_data,
                partial_address_nibbles,
            },
        }
        .prove();

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let branch_node_chain_target =
            builder.recursively_verify_constant_proof(&branch_node_chain_proof, &mut witness);
        let hashed_leaf_parser_target =
            builder.recursively_verify_constant_proof(&hashed_leaf_parser_proof, &mut witness);

        branch_node_chain_target
            .leaf_hash
            .connect(&hashed_leaf_parser_target.node_hash, &mut builder);

        branch_node_chain_target
            .partial_address
            .connect(&hashed_leaf_parser_target.partial_address, &mut builder);

        StorageTrieProofTarget {
            root_hash: branch_node_chain_target.root_hash,
            data_hash: hashed_leaf_parser_target.storage_data_hash,
            address: hashed_leaf_parser_target.final_address,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
