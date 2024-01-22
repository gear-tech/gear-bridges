use plonky2::plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig};

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{
            impl_target_set, BitArrayTarget, Ed25519PublicKeyTarget, Sha256Target,
            Sha256TargetGoldilocks, SingleTarget, TargetSetOperations, ValidatorSetTargetSet,
        },
        ProofCompositionBuilder, ProofCompositionTargets, TargetSet,
    },
    consts::VALIDATOR_COUNT,
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

impl_target_set! {
    pub struct NextValidatorSetTarget {
        validator_set_hash: Sha256TargetGoldilocks,
        next_validator_set_hash: Sha256TargetGoldilocks,
        authority_set_id: SingleTarget,
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

        let next_validator_set_data = self.next_validator_set_inclusion_proof.leaf_data;
        let mut next_validator_set = vec![];
        // REFACTOR
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

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let composition_builder = ProofCompositionBuilder::new_with_config(
            validator_set_hash_proof,
            non_hashed_next_validator_set_proof,
            config,
        );

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          targets: ProofCompositionTargets<_, _>| {
            let validator_set_hash_public_inputs: ValidatorSetHashTarget =
                targets.first_proof_public_inputs;
            let next_validator_set_public_inputs: NextValidatorSetNonHashedTarget =
                targets.second_proof_public_inputs;

            validator_set_hash_public_inputs.validator_set.connect(
                &next_validator_set_public_inputs.next_validator_set,
                builder,
            );

            NextValidatorSetTarget {
                validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                    next_validator_set_public_inputs.current_validator_set_hash,
                    builder,
                ),
                next_validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                    validator_set_hash_public_inputs.hash,
                    builder,
                ),
                authority_set_id: next_validator_set_public_inputs.authority_set_id,
            }
        };

        composition_builder.build(targets_op)
    }
}

impl_target_set! {
    struct NextValidatorSetNonHashedTarget {
        current_validator_set_hash: Sha256Target,
        authority_set_id: SingleTarget,
        next_validator_set: ValidatorSetTargetSet,
    }
}

impl_target_set! {
    struct ValidatorSessionKeysInStorageTarget {
        _session_key: Ed25519PublicKeyTarget,
        _babe_key: Ed25519PublicKeyTarget,
        pub grandpa_key: Ed25519PublicKeyTarget,
        _imonline_key: Ed25519PublicKeyTarget,
        _authoryty_discovery_key: Ed25519PublicKeyTarget,
    }
}

// REFACTOR
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

    fn into_targets_iter(self) -> impl Iterator<Item = plonky2::iop::target::Target> {
        self._length.into_targets_iter().chain(
            self.validators
                .into_iter()
                .flat_map(|v| v.into_targets_iter()),
        )
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

            // REFACTOR: implement for target itself.
            let validator_keys_targets = ValidatorSetInStorageTarget::parse(
                &mut merkle_proof_public_inputs
                    .leaf_data
                    .clone()
                    .into_iter()
                    .map(|t| t.target),
            )
            .validators
            .into_iter()
            .flat_map(|v| v.grandpa_key.into_targets_iter())
            .collect::<Vec<_>>();

            block_finality_public_inputs
                .message
                .block_hash
                .connect(&merkle_proof_public_inputs.root_hash, builder);

            NextValidatorSetNonHashedTarget {
                current_validator_set_hash: block_finality_public_inputs.validator_set_hash,
                authority_set_id: SingleTarget::from_u64_bits_le_lossy(
                    *block_finality_public_inputs.message.authority_set_id,
                    builder,
                ),
                next_validator_set: ValidatorSetTargetSet::parse(
                    &mut validator_keys_targets.into_iter(),
                ),
            }
        };

        composition_builder.build(targets_op)
    }
}
