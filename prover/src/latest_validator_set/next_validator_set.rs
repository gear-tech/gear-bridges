use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    block_finality::BlockFinality,
    common::{
        array_to_bits,
        targets::{
            impl_target_set, Blake2Target, Blake2TargetGoldilocks, TargetBitOperations, TargetSet,
        },
        BuilderExt,
    },
    prelude::*,
    storage_inclusion::StorageInclusion,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct ValidatorSetStorageItemTarget {
        pub validator_set_hash: Blake2Target,
    }
}

impl ValidatorSetStorageItemTarget {
    fn hash(&self, builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let bit_targets = self
            .clone()
            .into_targets_iter()
            .map(|t| BoolTarget::new_unsafe(t))
            .collect::<Vec<_>>();
        let mut hash_targets =
            plonky2_blake2b256::circuit::blake2_circuit_from_targets(builder, bit_targets)
                .into_iter()
                .map(|t| t.target);

        Blake2Target::parse_exact(&mut hash_targets)
    }
}

impl_target_set! {
    pub struct NextValidatorSetTarget {
        pub current_validator_set_hash: Blake2TargetGoldilocks,
        pub current_authority_set_id: Target,
        pub next_validator_set: Blake2TargetGoldilocks,
    }
}

pub struct NextValidatorSet {
    pub current_epoch_block_finality: BlockFinality,
    pub next_validator_set_inclusion_proof: StorageInclusion,
    pub next_validator_set_storage_data: Vec<u8>,
}

impl NextValidatorSet {
    pub fn prove(self) -> ProofWithCircuitData<NextValidatorSetTarget> {
        log::info!("Proving validator set change...");

        let storage_data_bits = array_to_bits(&self.next_validator_set_storage_data);

        let inclusion_proof = self.next_validator_set_inclusion_proof.prove();
        let block_finality_proof = self.current_epoch_block_finality.prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let inclusion_proof_target =
            builder.recursively_verify_constant_proof(&inclusion_proof, &mut witness);
        let block_finality_target =
            builder.recursively_verify_constant_proof(&block_finality_proof, &mut witness);

        inclusion_proof_target
            .block_hash
            .connect(&block_finality_target.message.block_hash, &mut builder);

        let authority_set_id = Target::from_u64_bits_le_lossy(
            block_finality_target.message.authority_set_id,
            &mut builder,
        );

        let mut storage_data_bit_targets = storage_data_bits.into_iter().map(|bit| {
            let target = builder.add_virtual_bool_target_safe();
            witness.set_bool_target(target, bit);
            target.target
        });
        let storage_data_target =
            ValidatorSetStorageItemTarget::parse_exact(&mut storage_data_bit_targets);

        let storage_data_hash_target = storage_data_target.hash(&mut builder);
        storage_data_hash_target.connect(&inclusion_proof_target.storage_item_hash, &mut builder);

        NextValidatorSetTarget {
            current_validator_set_hash: Blake2TargetGoldilocks::from_blake2_target(
                block_finality_target.validator_set_hash,
                &mut builder,
            ),
            current_authority_set_id: authority_set_id,
            next_validator_set: Blake2TargetGoldilocks::from_blake2_target(
                storage_data_target.validator_set_hash,
                &mut builder,
            ),
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
