//! ### Circuit that's used to prove that some block was finalized.
//!
//! In order to keep `VerifierOnlyCircuitData` constant this circuit exposes blake2b hash of
//! concatenated validator set keys instead of validator set itself.
//!
//! NOTE: This circuit decides that block is finalized when more than 2/3 of validator set have
//! signed it.

use plonky2::{
    iop::{target::Target, witness::PartialWitness},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    common::{
        targets::{
            impl_parsable_target_set, impl_target_set, BitArrayTarget, Blake2Target,
            TargetBitOperations, TargetSet,
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
    /// Public inputs for `BlockFinality`.
    pub struct BlockFinalityTarget {
        /// Blake2 hash of concatenated validator public keys.
        pub validator_set_hash: Blake2Target,
        /// GRANDPA message.
        pub message: GrandpaVoteTarget,
    }
}

// Assume the layout for vote:
// - enum discriminant(1 for pre-commit)    (1 byte)
// - block hash                             (32 bytes)
// - block number                           (4 bytes)
// - round number                           (8 bytes)
// - authority set id                       (8 bytes)
// TODO: Rename to GrandpaMessageTarget
impl_parsable_target_set! {
    /// Target that reflects the way GRANDPA vote is implemented in substrate.
    pub struct GrandpaVoteTarget {
        /// Discriminant determining sub-round of voting. 1 here stands for pre-commit.
        pub discriminant: BitArrayTarget<8>,
        /// Block hash that's being finalized.
        pub block_hash: Blake2Target,
        /// Block number that's being finalized.
        pub block_number: BitArrayTarget<32>,
        _round_number: BitArrayTarget<64>,
        /// Current GRANDPA authority set id.
        pub authority_set_id: BitArrayTarget<64>,
    }
}

/// Pre-commit data that's used to prove validator signs.
#[derive(Clone)]
pub struct PreCommit {
    /// Public key of validator this pre-commit belongs to.
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    /// Signature casted by validator this pre-commit belongs to.
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
    /// Actual validator set for current authority set id.
    pub validator_set: Vec<[u8; consts::ED25519_PUBLIC_KEY_SIZE]>,
    /// Pre-commits casted from GRANDPA voters. To successfully prove block finalization there
    /// should be at least > 2/3 signatures of entire validator set. These pre-commits can be
    /// gathered using grandpa_proveFinality RPC call.
    pub pre_commits: Vec<PreCommit>,
    /// Message that GRANDPA voters sign.
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl BlockFinality {
    pub(crate) fn prove(self) -> ProofWithCircuitData<BlockFinalityTarget> {
        log::debug!("Proving block finality...");

        // Find such a number that processed_validator_count > 2/3 * validator_count.
        let processed_validator_count = (2 * self.validator_set.len()) / 3 + 1;

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

        let message = validator_signs_target.message;
        let discriminant = Target::from_bool_targets_le(message.discriminant, &mut builder);
        let pre_commit_discriminant = builder.one();
        builder.connect(discriminant, pre_commit_discriminant);

        BlockFinalityTarget {
            validator_set_hash: validator_signs_target.validator_set_hash,
            message,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
