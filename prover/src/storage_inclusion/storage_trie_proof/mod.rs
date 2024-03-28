use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    common::{
        targets::{impl_target_set, Blake2Target, ParsableTargetSet, TargetSet},
        ProofComposition,
    },
    prelude::*,
    ProofWithCircuitData,
};

use super::BranchNodeData;

use self::{
    branch_node_chain::BranchNodeChainParserTarget,
    hashed_leaf_parser::{HashedLeafParser, HashedLeafParserTarget},
    node_parser::leaf_parser::LeafParser,
    storage_address::PartialStorageAddressTarget,
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
                &mut branch_node_chain_proof.pis().into_iter(),
            );
            let partial_address = branch_node_chain_pis.partial_address;
            partial_address.address[..partial_address.length as usize].to_vec()
        };

        let hashed_leaf_parser_proof = HashedLeafParser {
            leaf_parser: LeafParser {
                node_data: self.leaf_node_data,
                partial_address_nibbles,
            },
        }
        .prove();

        let composition_builder =
            ProofComposition::new(branch_node_chain_proof, hashed_leaf_parser_proof);

        let targets_op =
            |builder: &mut CircuitBuilder<F, D>,
             branch_node_chain_proof: BranchNodeChainParserTarget,
             hashed_leaf_parser_proof: HashedLeafParserTarget| {
                branch_node_chain_proof
                    .leaf_hash
                    .connect(&hashed_leaf_parser_proof.node_hash, builder);

                branch_node_chain_proof
                    .partial_address
                    .connect(&hashed_leaf_parser_proof.partial_address, builder);

                StorageTrieProofTarget {
                    root_hash: branch_node_chain_proof.root_hash,
                    data_hash: hashed_leaf_parser_proof.storage_data_hash,
                    address: hashed_leaf_parser_proof.final_address,
                }
            };

        composition_builder.compose(targets_op)
    }
}
