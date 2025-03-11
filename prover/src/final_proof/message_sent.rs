//! ### Circuit that's used to prove that message was queued for relaying.

use anyhow::Result;
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
            impl_target_set, ArrayTarget, Blake2Target, Blake2TargetGoldilocks,
            MessageTargetGoldilocks, TargetBitOperations, TargetSet,
        },
        BuilderExt, ProofWithCircuitData,
    },
    consts::MESSAGE_SIZE_IN_BITS,
    prelude::*,
    storage_inclusion::StorageInclusion,
};

impl_target_set! {
    /// Public inputs for `MessageSent`.
    pub struct MessageSentTarget {
        /// Blake2 hash of concatenated validator public inputs.
        pub validator_set_hash: Blake2TargetGoldilocks,
        /// Actual GRANDPA authority set id.
        pub authority_set_id: Target,
        /// Block number where message was sent.
        pub block_number: Target,
        /// Contents of the message that gets relayed.
        pub message_contents: MessageTargetGoldilocks,
    }
}

impl_target_set! {
    struct MessageInStorageTarget {
        merkle_trie_root: ArrayTarget<BoolTarget, MESSAGE_SIZE_IN_BITS>,
    }
}

impl MessageInStorageTarget {
    fn hash(&self, builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let bit_targets = self
            .clone()
            .into_targets_iter()
            .map(BoolTarget::new_unsafe)
            .collect::<Vec<_>>();
        let mut hash_targets =
            plonky2_blake2b256::circuit::blake2_circuit_from_targets(builder, bit_targets)
                .into_iter()
                .map(|t| t.target);

        Blake2Target::parse_exact(&mut hash_targets)
    }
}

pub struct MessageSent {
    /// Proof that block where message is present in storage is finalized.
    pub block_finality: BlockFinality,
    /// Proof that message is present in the storage.
    pub inclusion_proof: StorageInclusion,
    /// Original data stored in substrate storage.
    pub message_storage_data: Vec<u8>,
}

impl MessageSent {
    pub fn prove(self) -> Result<ProofWithCircuitData<MessageSentTarget>> {
        log::debug!("Proving message presence in finalized block...");

        let inclusion_proof = self.inclusion_proof.prove()?;
        let finality_proof = self.block_finality.prove()?;

        log::debug!("Composing inclusion and finality proofs...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let inclusion_proof_target =
            builder.recursively_verify_constant_proof(&inclusion_proof, &mut witness)?;
        let finality_proof_target =
            builder.recursively_verify_constant_proof(&finality_proof, &mut witness)?;

        let block_number =
            Target::from_bool_targets_le(finality_proof_target.message.block_number, &mut builder);

        inclusion_proof_target
            .block_hash
            .connect(&finality_proof_target.message.block_hash, &mut builder);

        let storage_data_bits = array_to_bits(&self.message_storage_data);
        let mut storage_data_bit_targets = storage_data_bits
            .into_iter()
            .map(|bit| {
                let target = builder.add_virtual_bool_target_safe();
                witness.set_bool_target(target, bit)?;
                Ok(target.target)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter();
        let storage_data_target =
            MessageInStorageTarget::parse_exact(&mut storage_data_bit_targets);

        storage_data_target
            .hash(&mut builder)
            .connect(&inclusion_proof_target.storage_item_hash, &mut builder);

        MessageSentTarget {
            validator_set_hash: Blake2TargetGoldilocks::from_blake2_target(
                finality_proof_target.validator_set_hash,
                &mut builder,
            ),
            authority_set_id: Target::from_u64_bits_le_lossy(
                finality_proof_target.message.authority_set_id,
                &mut builder,
            ),
            block_number,
            message_contents: MessageTargetGoldilocks::from_bit_array(
                storage_data_target.merkle_trie_root,
                &mut builder,
            ),
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
