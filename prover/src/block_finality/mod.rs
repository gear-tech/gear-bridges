use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{
            impl_parsable_target_set, impl_target_set, BitArrayTarget, Blake2Target, Sha256Target,
            TargetSet,
        },
        BuilderExt,
    },
    consts::{GRANDPA_VOTE_LENGTH, PROCESSED_VALIDATOR_COUNT, VALIDATOR_COUNT},
    prelude::*,
    ProofWithCircuitData,
};

mod indexed_validator_sign;
pub mod validator_set_hash;
mod validator_signs_chain;

use validator_set_hash::ValidatorSetHash;
use validator_signs_chain::ValidatorSignsChain;

use self::validator_set_hash::ValidatorSetHashTarget;

impl_target_set! {
    pub struct BlockFinalityTarget {
        pub validator_set_hash: Sha256Target,
        pub message: GrandpaVoteTarget,
    }
}

// Assume the layout for vote:
// - ???                    (1 byte)
// - block hash             (32 bytes)
// - block number           (4 bytes)
// - round number           (8 bytes)
// - authority set id       (8 bytes)
impl_parsable_target_set! {
    pub struct GrandpaVoteTarget {
        _aux_data: BitArrayTarget<8>,
        pub block_hash: Blake2Target,
        _aux_data_2: BitArrayTarget<96>,
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
    pub validator_set: [[u8; consts::ED25519_PUBLIC_KEY_SIZE]; VALIDATOR_COUNT],
    pub pre_commits: Vec<PreCommit>,
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl BlockFinality {
    pub fn prove(&self) -> ProofWithCircuitData<BlockFinalityTarget> {
        log::info!("Proving block finality...");

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
            .take(PROCESSED_VALIDATOR_COUNT)
            .collect();

        assert_eq!(processed_pre_commits.len(), PROCESSED_VALIDATOR_COUNT);

        let validator_set_hash_proof = ValidatorSetHash {
            validator_set: self.validator_set,
        }
        .prove();

        let validator_signs_proof = ValidatorSignsChain {
            validator_set_hash_proof,
            pre_commits: processed_pre_commits,
            message: self.message,
        }
        .prove();

        log::info!("Composing validator signs and validator set hash proofs...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let validator_signs_target =
            builder.recursively_verify_constant_proof(&validator_signs_proof, &mut witness);

        BlockFinalityTarget {
            validator_set_hash: validator_signs_target.validator_set_hash,
            message: validator_signs_target.message,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
