use plonky2::{
    iop::witness::PartialWitness,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    block_finality::BlockFinality,
    common::{
        targets::{
            impl_target_set, BitArrayTarget, MessageTargetGoldilocks, Sha256TargetGoldilocks,
            SingleTarget, TargetSet,
        },
        BuilderExt,
    },
    storage_inclusion::StorageInclusion,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct MessageSentTarget {
        pub validator_set_hash: Sha256TargetGoldilocks,
        pub authority_set_id: SingleTarget,
        pub message_contents: MessageTargetGoldilocks,
    }
}

pub struct MessageSent {
    pub block_finality: BlockFinality,
    pub inclusion_proof: StorageInclusion,
}

impl MessageSent {
    pub fn prove(self) -> ProofWithCircuitData<MessageSentTarget> {
        log::info!("Proving message presense in finalized block...");

        let inclusion_proof = self.inclusion_proof.prove();
        let finality_proof = self.block_finality.prove();

        log::info!("Composing inclusion and finality proofs...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let inclusion_proof_target =
            builder.recursively_verify_constant_proof(inclusion_proof, &mut witness);
        let finality_proof_target =
            builder.recursively_verify_constant_proof(finality_proof, &mut witness);

        inclusion_proof_target
            .block_hash
            .connect(&finality_proof_target.message.block_hash, &mut builder);

        // TODO: De-hash item here.
        let padding_targets = (0..4).map(|_| builder.constant_bool(false).target);
        let message_targets = BitArrayTarget::<260>::parse(
            &mut inclusion_proof_target
                .storage_item_hash
                .into_targets_iter()
                .take(256)
                .chain(padding_targets),
        );

        MessageSentTarget {
            validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                finality_proof_target.validator_set_hash,
                &mut builder,
            ),
            authority_set_id: SingleTarget::from_u64_bits_le_lossy(
                finality_proof_target.message.authority_set_id,
                &mut builder,
            ),
            message_contents: MessageTargetGoldilocks::from_bit_array(
                message_targets,
                &mut builder,
            ),
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }
}
