use plonky2::plonk::circuit_builder::CircuitBuilder;

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{
            BitArrayTarget, Ed25519PublicKeyTarget, Sha256Target, Sha256TargetGoldilocks,
            SingleTarget, TargetSetOperations,
        },
        ProofCompositionBuilder, ProofCompositionTargets, TargetSet,
    },
    consts::{ED25519_PUBLIC_KEY_SIZE_IN_BITS, VALIDATOR_COUNT},
    merkle_proof::{MerkleProof, MerkleProofTarget},
    prelude::*,
    validator_set_hash::{ValidatorSetHash, ValidatorSetHashTarget},
    ProofWithCircuitData,
};

// record for each validator: (AccountId, SessionKeys)
// SessionKeys = (Babe, Grandpa, ImOnline, AuthorityDiscovery)
const SESSION_KEYS_SIZE: usize = 5 * 32;
const SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE: usize = 1 + VALIDATOR_COUNT * SESSION_KEYS_SIZE;
const SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE_IN_BITS: usize =
    SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE * 8;

#[derive(Clone)]
pub struct NextValidatorSetTarget {
    validator_set_hash: Sha256TargetGoldilocks,
    next_validator_set_hash: Sha256TargetGoldilocks,
    authority_set_id: SingleTarget,
}

impl TargetSet for NextValidatorSetTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            validator_set_hash: Sha256TargetGoldilocks::parse(raw),
            next_validator_set_hash: Sha256TargetGoldilocks::parse(raw),
            authority_set_id: SingleTarget::parse(raw),
        }
    }
}

pub struct NextValidatorSet {
    pub current_epoch_block_finality: BlockFinality,
    pub next_validator_set_inclusion_proof:
        MerkleProof<SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE_IN_BITS>,
}

impl NextValidatorSet {
    pub fn prove(&self) -> ProofWithCircuitData<NextValidatorSetTarget> {
        log::info!("Proving validator set hash change...");

        let next_validator_set_data = self.next_validator_set_inclusion_proof.leaf_data.clone();
        let mut next_validator_set = vec![];
        for validator_idx in 0..VALIDATOR_COUNT {
            next_validator_set.push(
                next_validator_set_data[1
                    + validator_idx * SESSION_KEYS_SIZE
                    + consts::ED25519_PUBLIC_KEY_SIZE * 2
                    ..1 + validator_idx * SESSION_KEYS_SIZE + consts::ED25519_PUBLIC_KEY_SIZE * 3]
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

            next_validator_set_public_inputs
                .authority_set_id
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
    authority_set_id: SingleTarget,
    next_validator_set: [Ed25519PublicKeyTarget; VALIDATOR_COUNT],
}

impl TargetSet for NextValidatorSetNonHashedTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            current_validator_set_hash: Sha256Target::parse(raw),
            authority_set_id: SingleTarget::parse(raw),
            next_validator_set: (0..VALIDATOR_COUNT)
                .map(|_| Ed25519PublicKeyTarget::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }
}

#[derive(Clone, Debug)]
struct ValidatorSessionKeysInStorageTarget {
    _session_key: Ed25519PublicKeyTarget,
    _babe_key: Ed25519PublicKeyTarget,
    pub grandpa_key: Ed25519PublicKeyTarget,
    _imonline_key: Ed25519PublicKeyTarget,
    _authoryty_discovery_key: Ed25519PublicKeyTarget,
}

impl TargetSet for ValidatorSessionKeysInStorageTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            _session_key: TargetSet::parse(raw),
            _babe_key: TargetSet::parse(raw),
            grandpa_key: TargetSet::parse(raw),
            _imonline_key: TargetSet::parse(raw),
            _authoryty_discovery_key: TargetSet::parse(raw),
        }
    }
}

#[derive(Clone)]
struct ValidatorSetInStorageTarget {
    _length: BitArrayTarget<8>,
    validators: [ValidatorSessionKeysInStorageTarget; VALIDATOR_COUNT],
}

impl TargetSet for ValidatorSetInStorageTarget {
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            _length: BitArrayTarget::parse(raw),
            validators: (0..VALIDATOR_COUNT)
                .map(|_| ValidatorSessionKeysInStorageTarget::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }
}

struct NextValidatorSetNonHashed {
    current_epoch_block_finality: BlockFinality,
    next_validator_set_inclusion_proof:
        MerkleProof<SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE_IN_BITS>,
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
                SESSION_KEYS_ALL_VALIDATORS_SIZE_IN_STORAGE_IN_BITS,
            > = targets.first_proof_public_inputs;
            let block_finality_public_inputs: BlockFinalityTarget =
                targets.second_proof_public_inputs;

            block_finality_public_inputs
                .validator_set_hash
                .register_as_public_inputs(builder);

            SingleTarget::from_u64_bits_le_lossy(
                *block_finality_public_inputs.message.authority_set_id,
                builder,
            )
            .register_as_public_inputs(builder);

            let validator_set_targets = ValidatorSetInStorageTarget::parse(
                &mut merkle_proof_public_inputs
                    .leaf_data
                    .clone()
                    .into_iter()
                    .map(|t| t.target),
            );

            for validator_pk in &validator_set_targets.validators {
                validator_pk.grandpa_key.register_as_public_inputs(builder);
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
