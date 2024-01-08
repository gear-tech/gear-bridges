use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{
            BitArrayTarget, Ed25519PublicKeyTarget, Sha256Target, Sha256TargetGoldilocks,
            TargetSetOperations,
        },
        ProofCompositionBuilder, ProofCompositionTargets, TargetSet,
    },
    consts::{ED25519_PUBLIC_KEY_SIZE_IN_BITS, VALIDATOR_COUNT},
    merkle_proof::{MerkleProof, MerkleProofTarget},
    prelude::*,
    validator_set_hash::{ValidatorSetHash, ValidatorSetHashTarget},
    ProofWithCircuitData,
};

const VALIDATOR_SET_SIZE_IN_STORAGE_IN_BITS: usize =
    8 + VALIDATOR_COUNT * (ED25519_PUBLIC_KEY_SIZE_IN_BITS + AUTHORITY_WEIGHT_SIZE_IN_BITS);

const AUTHORITY_WEIGHT_SIZE: usize = 8;
const AUTHORITY_WEIGHT_SIZE_IN_BITS: usize = AUTHORITY_WEIGHT_SIZE * 8;

#[derive(Clone)]
pub struct NextValidatorSetTarget {
    validator_set_hash: Sha256TargetGoldilocks,
    next_validator_set_hash: Sha256TargetGoldilocks,
}

impl TargetSet for NextValidatorSetTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            validator_set_hash: Sha256TargetGoldilocks::parse(raw),
            next_validator_set_hash: Sha256TargetGoldilocks::parse(raw),
        }
    }
}

pub struct NextValidatorSet {
    pub current_epoch_block_finality: BlockFinality,
    pub next_validator_set_inclusion_proof: MerkleProof<VALIDATOR_SET_SIZE_IN_STORAGE_IN_BITS>,
}

impl NextValidatorSet {
    pub fn prove(&self) -> ProofWithCircuitData<NextValidatorSetTarget> {
        log::info!("Proving validator set hash change...");

        let next_validator_set_data = self.next_validator_set_inclusion_proof.leaf_data.clone();
        let next_validator_set_len = (next_validator_set_data.len() - 1)
            / (consts::ED25519_PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE);
        let mut next_validator_set = vec![];
        for validator_idx in 0..next_validator_set_len {
            next_validator_set.push(
                next_validator_set_data[1 + validator_idx
                    * (consts::ED25519_PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE)
                    ..1 + validator_idx
                        * (consts::ED25519_PUBLIC_KEY_SIZE + AUTHORITY_WEIGHT_SIZE)
                        + consts::ED25519_PUBLIC_KEY_SIZE]
                    .try_into()
                    .unwrap(),
            );
        }

        let validator_set_hash_proof = ValidatorSetHash {
            validator_set: next_validator_set.try_into().unwrap(),
        }
        .prove();

        let non_hashed_next_validator_set_proof = NextValidatorSetNonHashed {
            current_epoch_block_finality: self.current_epoch_block_finality.clone(),
            next_validator_set_inclusion_proof: self.next_validator_set_inclusion_proof.clone(),
        }
        .prove();

        let composition_builder = ProofCompositionBuilder::new(
            validator_set_hash_proof,
            non_hashed_next_validator_set_proof,
        );

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          targets: ProofCompositionTargets<_, _>| {
            let validator_set_hash_public_inputs: ValidatorSetHashTarget =
                targets.first_proof_public_inputs;
            let next_validator_set_public_inputs: NextValidatorSetNonHashedTarget =
                targets.second_proof_public_inputs;

            Sha256TargetGoldilocks::from_sha256_target(
                next_validator_set_public_inputs.current_validator_set_hash,
                builder,
            )
            .register_as_public_inputs(builder);

            Sha256TargetGoldilocks::from_sha256_target(
                validator_set_hash_public_inputs.hash,
                builder,
            )
            .register_as_public_inputs(builder);

            for (validator_1, validator_2) in validator_set_hash_public_inputs
                .validator_set
                .iter()
                .zip(next_validator_set_public_inputs.next_validator_set.iter())
            {
                validator_1.connect(validator_2, builder);
            }
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}

#[derive(Clone)]
struct NextValidatorSetNonHashedTarget {
    current_validator_set_hash: Sha256Target,
    next_validator_set: [Ed25519PublicKeyTarget; VALIDATOR_COUNT],
}

impl TargetSet for NextValidatorSetNonHashedTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            current_validator_set_hash: Sha256Target::parse(raw),
            next_validator_set: (0..VALIDATOR_COUNT)
                .map(|_| Ed25519PublicKeyTarget::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }
}

// Assume the layout of leaf data:
// - vector length          (1 byte)
// - public key #1          (32 bytes)
// - authority weight #1    (8 bytes)
// - public key #2          (32 bytes)
// ...
// - authority weight #N    (8 bytes)
#[derive(Clone)]
struct ValidatorSetInStorageTarget {
    _length: BitArrayTarget<8>,
    validators: [(
        Ed25519PublicKeyTarget,
        BitArrayTarget<AUTHORITY_WEIGHT_SIZE_IN_BITS>,
    ); VALIDATOR_COUNT],
}

impl TargetSet for ValidatorSetInStorageTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            _length: BitArrayTarget::parse(raw),
            validators: (0..VALIDATOR_COUNT)
                .map(|_| {
                    (
                        Ed25519PublicKeyTarget::parse(raw),
                        BitArrayTarget::parse(raw),
                    )
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }
}

struct NextValidatorSetNonHashed {
    current_epoch_block_finality: BlockFinality,
    next_validator_set_inclusion_proof: MerkleProof<VALIDATOR_SET_SIZE_IN_STORAGE_IN_BITS>,
}

impl NextValidatorSetNonHashed {
    pub fn prove(&self) -> ProofWithCircuitData<NextValidatorSetNonHashedTarget> {
        log::info!("Proving validator set change...");

        let merkle_tree_proof = self.next_validator_set_inclusion_proof.prove();
        let block_finality_proof = self.current_epoch_block_finality.prove();

        let composition_builder =
            ProofCompositionBuilder::new(merkle_tree_proof, block_finality_proof);

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          targets: ProofCompositionTargets<_, _>| {
            let merkle_proof_public_inputs: MerkleProofTarget<
                VALIDATOR_SET_SIZE_IN_STORAGE_IN_BITS,
            > = targets.first_proof_public_inputs;
            let block_finality_public_inputs: BlockFinalityTarget =
                targets.second_proof_public_inputs;

            block_finality_public_inputs
                .validator_set_hash
                .register_as_public_inputs(builder);

            let validator_set_targets = ValidatorSetInStorageTarget::parse(
                &mut merkle_proof_public_inputs
                    .leaf_data
                    .clone()
                    .into_iter()
                    .map(|t| t.target),
            );

            for validator_pk in &validator_set_targets.validators {
                validator_pk.0.register_as_public_inputs(builder);
            }

            block_finality_public_inputs
                .message
                .block_hash
                .connect(&merkle_proof_public_inputs.root_hash, builder);
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}
