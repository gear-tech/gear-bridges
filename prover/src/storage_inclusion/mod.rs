use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{impl_parsable_target_set, Blake2Target, TargetSet},
        BuilderExt,
    },
    prelude::*,
    ProofWithCircuitData,
};

mod block_header_parser;
mod generic_hasher;
mod storage_trie_proof;

use storage_trie_proof::storage_address::PartialStorageAddressTarget;

use self::{block_header_parser::BlockHeaderParser, storage_trie_proof::StorageTrieProof};

impl_parsable_target_set! {
    pub struct StorageInclusionTarget {
        pub block_hash: Blake2Target,
        pub storage_item_hash: Blake2Target,
        pub address: PartialStorageAddressTarget
    }
}

#[derive(Clone)]

pub struct BranchNodeData {
    data: Vec<u8>,
    child_nibble: u8,
}

#[derive(Clone)]
pub struct StorageInclusion {
    pub block_header_data: Vec<u8>,
    pub branch_node_data: Vec<BranchNodeData>,
    pub leaf_node_data: Vec<u8>,
}

impl StorageInclusion {
    pub fn prove(self) -> ProofWithCircuitData<StorageInclusionTarget> {
        let block_header_proof = BlockHeaderParser {
            header_data: self.block_header_data,
        }
        .prove();

        let storage_trie_proof = StorageTrieProof {
            branch_nodes: self.branch_node_data,
            leaf_node_data: self.leaf_node_data,
        }
        .prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let block_header_target =
            builder.recursively_verify_constant_proof(block_header_proof, &mut witness);
        let storage_trie_target =
            builder.recursively_verify_constant_proof(storage_trie_proof, &mut witness);

        block_header_target
            .state_root
            .connect(&storage_trie_target.root_hash, &mut builder);

        StorageInclusionTarget {
            block_hash: block_header_target.block_hash,
            storage_item_hash: storage_trie_target.data_hash,
            address: storage_trie_target.address,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
