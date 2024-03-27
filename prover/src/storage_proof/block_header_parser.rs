use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{impl_parsable_target_set, ArrayTarget, Blake2Target, ByteTarget, TargetSet},
        BuilderExt,
    },
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    ProofWithCircuitData,
};

use super::generic_hasher::GenericBlake2;

// Block header have the folowing structure:
// - previous block hash    (32 bytes)
// - block number           (4 bytes)
// - state root             (32 bytes)
// - ...
const STATE_ROOT_OFFSET_IN_BLOCK_HEADER: usize = 32 + 4;

impl_parsable_target_set! {
    pub struct BlockHeaderParserTarget {
        pub block_hash: Blake2Target,
        pub state_root: Blake2Target,
    }
}

pub struct BlockHeaderParser {
    pub header_data: Vec<u8>,
}

impl BlockHeaderParser {
    pub fn prove(self) -> ProofWithCircuitData<BlockHeaderParserTarget> {
        let hasher_proof = GenericBlake2 {
            data: self.header_data,
        }
        .prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let hasher_target = builder.recursively_verify_constant_proof(hasher_proof, &mut witness);

        let state_root_bytes: ArrayTarget<ByteTarget, BLAKE2_DIGEST_SIZE> = hasher_target
            .data
            .constant_read_array(STATE_ROOT_OFFSET_IN_BLOCK_HEADER);
        let mut state_root_bits = state_root_bytes
            .0
            .into_iter()
            .map(|byte| {
                byte.to_bit_targets(&mut builder)
                    .0
                    .into_iter()
                    .map(|t| t.target)
                    .rev()
            })
            .flatten();

        let state_root = Blake2Target::parse_exact(&mut state_root_bits);

        BlockHeaderParserTarget {
            block_hash: hasher_target.hash,
            state_root,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
