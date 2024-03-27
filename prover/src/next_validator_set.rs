use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_field::types::PrimeField64;
use std::iter;

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        array_to_bits,
        targets::{
            impl_target_set, ArrayTarget, BitArrayTarget, Blake2Target, Ed25519PublicKeyTarget,
            Sha256Target, Sha256TargetGoldilocks, SingleTarget, TargetSet, ValidatorSetTarget,
        },
        BuilderExt, ProofComposition,
    },
    consts::VALIDATOR_COUNT,
    next_validator_set,
    prelude::*,
    storage_inclusion::{StorageInclusion, StorageInclusionTarget},
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
        pub validator_set_hash: Sha256TargetGoldilocks,
        pub next_validator_set_hash: Sha256TargetGoldilocks,
        pub current_authority_set_id: SingleTarget,
    }
}

pub struct NextValidatorSet {
    pub current_epoch_block_finality: BlockFinality,
    pub next_validator_set_inclusion_proof: StorageInclusion,
    pub next_validator_set_storage_data: Vec<u8>,
}

impl NextValidatorSet {
    pub fn prove(&self) -> ProofWithCircuitData<NextValidatorSetTarget> {
        log::info!("Proving validator set hash change...");

        let mut next_validator_set = vec![];
        // TODO REFACTOR
        for validator_idx in 0..VALIDATOR_COUNT {
            next_validator_set.push(
                self.next_validator_set_storage_data[1
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
            next_validator_set_storage_data: self.next_validator_set_storage_data.clone(),
        }
        .prove();

        let mut config = CircuitConfig::standard_recursion_config();
        // TODO: Can be removed as it's not the latest proof in chain for now.
        config.fri_config.cap_height = 0;
        let composition_builder = ProofComposition::new_with_config(
            validator_set_hash_proof,
            non_hashed_next_validator_set_proof,
            config,
        );

        let targets_op =
            |builder: &mut CircuitBuilder<F, D>,
             validator_set_hash: ValidatorSetHashTarget,
             next_validator_set: NextValidatorSetNonHashedTarget| {
                validator_set_hash
                    .validator_set
                    .connect(&next_validator_set.next_validator_set, builder);

                NextValidatorSetTarget {
                    validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                        next_validator_set.current_validator_set_hash,
                        builder,
                    ),
                    next_validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                        validator_set_hash.hash,
                        builder,
                    ),
                    current_authority_set_id: next_validator_set.authority_set_id,
                }
            };

        composition_builder
            .assert_both_circuit_digests()
            .compose(targets_op)
    }
}

impl_target_set! {
    struct NextValidatorSetNonHashedTarget {
        current_validator_set_hash: Sha256Target,
        authority_set_id: SingleTarget,
        next_validator_set: ValidatorSetTarget,
    }
}

impl_target_set! {
    struct SessionKeysTarget {
        _session_key: Ed25519PublicKeyTarget,
        _babe_key: Ed25519PublicKeyTarget,
        pub grandpa_key: Ed25519PublicKeyTarget,
        _imonline_key: Ed25519PublicKeyTarget,
        _authoryty_discovery_key: Ed25519PublicKeyTarget,
    }
}

impl_target_set! {
    struct ValidatorSetInStorageTarget {
        _length: BitArrayTarget<8>,
        validators: ArrayTarget<SessionKeysTarget, VALIDATOR_COUNT>,
    }
}

impl ValidatorSetInStorageTarget {
    fn into_grandpa_authority_keys(self) -> ValidatorSetTarget {
        ValidatorSetTarget::parse(
            &mut self
                .validators
                .0
                .into_iter()
                .flat_map(|v| v.grandpa_key.into_targets_iter()),
        )
    }
}

struct NextValidatorSetNonHashed {
    current_epoch_block_finality: BlockFinality,
    next_validator_set_inclusion_proof: StorageInclusion,
    next_validator_set_storage_data: Vec<u8>,
}

impl NextValidatorSetNonHashed {
    pub fn prove(self) -> ProofWithCircuitData<NextValidatorSetNonHashedTarget> {
        log::info!("Proving validator set change...");

        let next_validator_set_bits = array_to_bits(&self.next_validator_set_storage_data);

        let inclusion_proof = self.next_validator_set_inclusion_proof.prove();
        let block_finality_proof = self.current_epoch_block_finality.prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let inclusion_proof_target =
            builder.recursively_verify_constant_proof(inclusion_proof, &mut witness);
        let block_finality_target =
            builder.recursively_verify_constant_proof(block_finality_proof, &mut witness);

        inclusion_proof_target
            .block_hash
            .connect(&block_finality_target.message.block_hash, &mut builder);

        let authority_set_id = SingleTarget::from_u64_bits_le_lossy(
            block_finality_target.message.authority_set_id,
            &mut builder,
        );

        let next_validator_set_targets: Vec<_> = next_validator_set_bits
            .into_iter()
            .map(|bit| {
                let target = builder.add_virtual_bool_target_safe();
                witness.set_bool_target(target, bit);
                target
            })
            .collect();

        let next_validator_set_hash = plonky2_blake2b256::circuit::blake2_circuit_from_targets(
            &mut builder,
            next_validator_set_targets.clone(),
        );
        let next_validator_set_hash =
            Blake2Target::parse_exact(&mut next_validator_set_hash.into_iter().map(|t| t.target));

        next_validator_set_hash.connect(&inclusion_proof_target.storage_item_hash, &mut builder);

        let next_validator_set = ValidatorSetInStorageTarget::parse_exact(
            &mut next_validator_set_targets.into_iter().map(|t| t.target),
        )
        .into_grandpa_authority_keys();

        NextValidatorSetNonHashedTarget {
            current_validator_set_hash: block_finality_target.validator_set_hash,
            authority_set_id,
            next_validator_set,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
