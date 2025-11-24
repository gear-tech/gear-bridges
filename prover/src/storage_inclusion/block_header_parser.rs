//! ### Circuit that's used to extract data from block header.
//!
//! Extracts state root from encoded block header and asserts that block hash equals to claimed.

use crate::{
    common::{
        blake2::{CircuitTargets as Blake2CircuitTargets, MAX_DATA_BYTES},
        targets::{impl_parsable_target_set, ArrayTarget, Blake2Target, TargetSet},
        BuilderExt, ProofWithCircuitData,
    },
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    storage_inclusion::scale_compact_integer_parser::full::{
        define as define_full_int_parser, InputTarget as FullIntParserInput,
    },
};
use plonky2::{
    iop::{target::Target, witness::PartialWitness},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

// Block header have the folowing structure:
// - previous block hash    (32 bytes)
// - block number           (1-4 bytes)
// - state root             (32 bytes)
// - extrinsics root        (32 bytes)
// - digest                 (generic)
const BLOCK_NUMBER_OFFSET_IN_BLOCK_HEADER: usize = 32;
const MAX_BLOCK_NUMBER_DATA_LENGTH: usize = 4;

impl_parsable_target_set! {
    /// Public inputs for `BlockHeaderParser` circuit.
    pub struct BlockHeaderParserTarget {
        /// Block hash.
        pub block_hash: Blake2Target,
        /// Storage trie root.
        pub state_root: Blake2Target,
        /// Number of the block.
        pub block_number: Target,
    }
}

pub struct BlockHeaderParser {
    /// Encoded block header data.
    pub header_data: Vec<u8>,
}

impl BlockHeaderParser {
    pub fn prove(self) -> ProofWithCircuitData<BlockHeaderParserTarget> {
        let circuit = Blake2CircuitTargets::new();
        let hasher_proof = circuit.prove::<MAX_DATA_BYTES>(&self.header_data);

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let hasher_target = builder.recursively_verify_constant_proof(&hasher_proof, &mut witness);

        // Will not exceed the length as block number isn't last field in header.
        let block_number_targets = hasher_target
            .data
            .constant_read_array(BLOCK_NUMBER_OFFSET_IN_BLOCK_HEADER);
        // Parse block number just to get encoded length and compute offset of state root later.
        let parsed_block_number = define_full_int_parser(
            FullIntParserInput {
                padded_bytes: block_number_targets,
            },
            &mut builder,
        );

        const USEFUL_DATA_OFFSET: usize = MAX_BLOCK_NUMBER_DATA_LENGTH + BLAKE2_DIGEST_SIZE;
        let useful_header_data: ArrayTarget<_, USEFUL_DATA_OFFSET> = hasher_target
            .data
            .constant_read_array(BLOCK_NUMBER_OFFSET_IN_BLOCK_HEADER);

        let state_root_bytes: ArrayTarget<_, BLAKE2_DIGEST_SIZE> =
            useful_header_data.random_read_array(parsed_block_number.length, &mut builder);
        let mut state_root_bits = state_root_bytes.0.into_iter().flat_map(|byte| {
            byte.as_bit_targets(&mut builder)
                .0
                .into_iter()
                .map(|t| t.target)
                .rev()
        });

        let state_root = Blake2Target::parse_exact(&mut state_root_bits);

        BlockHeaderParserTarget {
            block_hash: hasher_target.hash,
            state_root,
            block_number: parsed_block_number.decoded,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
