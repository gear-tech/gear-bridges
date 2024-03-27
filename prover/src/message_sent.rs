use plonky2::plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig};

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{
            impl_target_set, BitArrayTarget, MessageTargetGoldilocks, Sha256TargetGoldilocks,
            SingleTarget, TargetSet,
        },
        ProofComposition,
    },
    prelude::*,
    storage_inclusion::{StorageInclusion, StorageInclusionTarget},
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

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let composition_builder =
            ProofComposition::new_with_config(inclusion_proof, finality_proof, config);

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          inclusion_proof: StorageInclusionTarget,
                          finality_proof: BlockFinalityTarget| {
            inclusion_proof
                .block_hash
                .connect(&finality_proof.message.block_hash, builder);

            // TODO: De-hash item here.
            let padding_targets = (0..4).map(|_| builder.constant_bool(false).target);
            let message_targets = BitArrayTarget::<260>::parse(
                &mut inclusion_proof
                    .storage_item_hash
                    .into_targets_iter()
                    .take(256)
                    .chain(padding_targets),
            );

            MessageSentTarget {
                validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                    finality_proof.validator_set_hash,
                    builder,
                ),
                authority_set_id: SingleTarget::from_u64_bits_le_lossy(
                    finality_proof.message.authority_set_id,
                    builder,
                ),
                message_contents: MessageTargetGoldilocks::from_bit_array(message_targets, builder),
            }
        };

        composition_builder
            .assert_both_circuit_digests()
            .compose(targets_op)
    }
}
