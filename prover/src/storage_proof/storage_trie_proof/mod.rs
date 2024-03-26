use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    common::{
        targets::{impl_target_set, Blake2Target, TargetSet},
        ProofComposition,
    },
    prelude::*,
    ProofWithCircuitData,
};

use self::{
    branch_node_chain::BranchNodeChainParserTarget,
    hashed_leaf_parser::{HashedLeafParser, HashedLeafParserTarget},
    storage_address::PartialStorageAddressTarget,
};

mod branch_node_chain;
mod generic_hasher;
mod hashed_branch_parser;
mod hashed_leaf_parser;
mod node_parser;
mod storage_address;

use branch_node_chain::BranchNodeChain;

impl_target_set! {
    pub struct StorageTrieProofTarget {
        pub root_hash: Blake2Target,
        pub data_hash: Blake2Target,
        pub address: PartialStorageAddressTarget
    }
}

pub struct StorageTrieProof {
    // TODO: Compose data from node vec.
    pub branch_node_chain: BranchNodeChain,
    pub hashed_leaf_parser: HashedLeafParser,
}

impl StorageTrieProof {
    pub fn prove(self) -> ProofWithCircuitData<StorageTrieProofTarget> {
        let branch_node_chain_proof = self.branch_node_chain.prove();
        let hashed_leaf_parser_proof = self.hashed_leaf_parser.prove();

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

        composition_builder
            .assert_both_circuit_digests()
            .compose(targets_op)
    }
}
