//! ### Circuit that's used to prove inclusion of some data into block storage.
//!
//! Currently it supports only `StorageValue` (not `StorageMap` and `StorageDoubleMap`) entries into
//! storage. Also trie node containing value must be either `Leaf` or `HashedValueLeaf`
//! (not `BranchWithValue`).
//!
//! In case data in leaf node is less or equal to 32 bytes in length, it gets inlined by substrate
//! storage logic. Currently we don't support inlined data with length != 32 bytes.
//!
//! All the above means that any data that's stored in `Leaf` or `HashedValueLeaf` will be parsed,
//! except ones that have length < 32 bytes.

use crate::{
    common::{
        targets::{impl_parsable_target_set, Blake2Target, TargetSet},
        BuilderExt, ProofWithCircuitData,
    },
    prelude::*,
};
use plonky2::{
    iop::{target::Target, witness::PartialWitness},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

mod block_header_parser;
mod scale_compact_integer_parser;
mod storage_trie_proof;

use storage_trie_proof::storage_address::StorageAddressTarget;

use self::{block_header_parser::BlockHeaderParser, storage_trie_proof::StorageTrieProof};

impl_parsable_target_set! {
    /// Public inputs for `StorageInclusion` proof.
    pub struct StorageInclusionTarget {
        /// Block hash where storage gets read.
        pub block_hash: Blake2Target,
        /// Blake2 hash of data included into storage. We don't use original data here as it'll
        /// potentially have generic length.
        pub storage_item_hash: Blake2Target,
        /// Block number where storage gets read.
        pub block_number: Target,
    }
}

#[derive(Clone)]
pub struct BranchNodeData {
    /// Encoded data for branch node.
    pub data: Vec<u8>,
    /// Address of the child node that leads to requested storage item.
    pub child_nibble: u8,
}

#[derive(Clone)]
pub struct StorageInclusion {
    /// Encoded block header.
    pub block_header_data: Vec<u8>,
    /// Encoded branch nodes, arranged from root to leaf.
    pub branch_node_data: Vec<BranchNodeData>,
    /// Encoded leaf node.
    pub leaf_node_data: Vec<u8>,
    /// Full storage item address. Note that nibble values are in range 0..=15.
    pub address_nibbles: Vec<u8>,
}

impl StorageInclusion {
    pub(crate) fn prove(self) -> ProofWithCircuitData<StorageInclusionTarget> {
        let block_header_proof = BlockHeaderParser {
            header_data: self.block_header_data,
        }
        .prove();

        let storage_trie_proof = StorageTrieProof {
            branch_nodes: self.branch_node_data,
            leaf_node_data: self.leaf_node_data,
        }
        .prove();

        log::debug!("Composing block header proof and storage trie proof...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let block_header_target =
            builder.recursively_verify_constant_proof(&block_header_proof, &mut witness);
        let storage_trie_target =
            builder.recursively_verify_constant_proof(&storage_trie_proof, &mut witness);

        block_header_target
            .state_root
            .connect(&storage_trie_target.root_hash, &mut builder);

        let storage_address = StorageAddressTarget::constant(self.address_nibbles, &mut builder);
        storage_address.connect(&storage_trie_target.address, &mut builder);

        StorageInclusionTarget {
            block_hash: block_header_target.block_hash,
            storage_item_hash: storage_trie_target.data_hash,
            block_number: block_header_target.block_number,
        }
        .register_as_public_inputs(&mut builder);

        let res = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Composed block header proof and storage trie proof");

        res
    }
}
