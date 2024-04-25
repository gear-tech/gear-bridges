use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{
            impl_parsable_target_set, impl_target_set, BitArrayTarget, Blake2Target, TargetSet,
        },
        BuilderExt, ProofWithCircuitData,
    },
    consts::GRANDPA_VOTE_LENGTH,
    prelude::*,
};

pub mod validator_set_hash;
mod validator_signs_chain;

use validator_set_hash::ValidatorSetHash;
use validator_signs_chain::ValidatorSignsChain;

impl_target_set! {
    pub struct BlockFinalityTarget {
        pub validator_set_hash: Blake2Target,
        pub message: GrandpaVoteTarget,
    }
}

// Assume the layout for vote:
// - enum discriminant(1 for pre-commit)    (1 byte)
// - block hash                             (32 bytes)
// - block number                           (4 bytes)
// - round number                           (8 bytes)
// - authority set id                       (8 bytes)
impl_parsable_target_set! {
    pub struct GrandpaVoteTarget {
        _aux_data: BitArrayTarget<8>,
        pub block_hash: Blake2Target,
        pub block_number: BitArrayTarget<32>,
        _aux_data_2: BitArrayTarget<64>,
        pub authority_set_id: BitArrayTarget<64>,
    }
}

#[derive(Clone)]
pub struct PreCommit {
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    pub signature: [u8; consts::ED25519_SIGNATURE_SIZE],
}

#[derive(Clone)]
struct ProcessedPreCommit {
    validator_idx: usize,
    public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    signature: [u8; consts::ED25519_SIGNATURE_SIZE],
}

#[derive(Clone)]
pub struct BlockFinality {
    pub validator_set: Vec<[u8; consts::ED25519_PUBLIC_KEY_SIZE]>,
    pub pre_commits: Vec<PreCommit>,
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl BlockFinality {
    pub(crate) fn prove(self) -> ProofWithCircuitData<BlockFinalityTarget> {
        log::debug!("Proving block finality...");

        // Find such a number that processed_validator_count > 2/3 * validator_count.
        let processed_validator_count = match self.validator_set.len() % 3 {
            0 | 1 => 2 * self.validator_set.len() / 3 + 1,
            2 => 2 * self.validator_set.len() / 3 + 2,
            _ => unreachable!(),
        };

        let processed_pre_commits: Vec<_> = self
            .pre_commits
            .iter()
            .filter_map(|pc| {
                let validator_idx = self.validator_set.iter().position(|v| v == &pc.public_key);
                validator_idx.map(|validator_idx| ProcessedPreCommit {
                    validator_idx,
                    public_key: pc.public_key,
                    signature: pc.signature,
                })
            })
            .take(processed_validator_count)
            .collect();

        assert_eq!(processed_pre_commits.len(), processed_validator_count);

        let validator_set_hash = ValidatorSetHash {
            validator_set: self.validator_set,
        };

        let validator_signs_proof = ValidatorSignsChain {
            validator_set_hash,
            pre_commits: processed_pre_commits,
            message: self.message,
        }
        .prove();

        log::debug!("Composing validator signs and validator set hash proofs...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let validator_signs_target =
            builder.recursively_verify_constant_proof(&validator_signs_proof, &mut witness);

        BlockFinalityTarget {
            validator_set_hash: validator_signs_target.validator_set_hash,
            message: validator_signs_target.message,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
