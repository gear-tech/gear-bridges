use plonky2::{
    iop::target::BoolTarget,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{
            impl_target_set, BitArrayTarget, MessageTargetGoldilocks, Sha256TargetGoldilocks,
            SingleTarget, TargetSetOperations,
        },
        ProofCompositionBuilder, ProofCompositionTargets, TargetSet,
    },
    merkle_proof::{MerkleProof, MerkleProofTarget},
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct MessageSentTarget {
        validator_set_hash: Sha256TargetGoldilocks,
        authority_set_id: SingleTarget,
        message_contents: MessageTargetGoldilocks,
    }
}

pub struct MessageSent<const MESSAGE_LENGTH_IN_BITS: usize>
where
    [(); MESSAGE_LENGTH_IN_BITS / 8]:,
{
    pub block_finality: BlockFinality,
    pub inclusion_proof: MerkleProof<MESSAGE_LENGTH_IN_BITS>,
}

impl<const MESSAGE_LENGTH_IN_BITS: usize> MessageSent<MESSAGE_LENGTH_IN_BITS>
where
    [(); MESSAGE_LENGTH_IN_BITS / 8]:,
{
    pub fn prove(&self) -> ProofWithCircuitData<MessageSentTarget> {
        log::info!("Proving message presense in finalized block...");

        let inclusion_proof = self.inclusion_proof.prove();
        let finality_proof = self.block_finality.prove();

        log::info!("Composing inclusion and finality proofs...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let composition_builder =
            ProofCompositionBuilder::new_with_config(inclusion_proof, finality_proof, config);

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          targets: ProofCompositionTargets<_, _>| {
            let inclusion_proof_public_inputs: MerkleProofTarget<MESSAGE_LENGTH_IN_BITS> =
                targets.first_proof_public_inputs;
            let finality_proof_public_inputs: BlockFinalityTarget =
                targets.second_proof_public_inputs;

            inclusion_proof_public_inputs
                .root_hash
                .connect(&finality_proof_public_inputs.message.block_hash, builder);

            // TODO: Assert here that provided leaf data have the correct size(Keccak256 size)
            // and pad it with zeroes.
            let message_targets_array: [BoolTarget; 260] = inclusion_proof_public_inputs.leaf_data
                [..260]
                .try_into()
                .unwrap();
            let message_targets: BitArrayTarget<260> = message_targets_array.into();

            MessageSentTarget {
                validator_set_hash: Sha256TargetGoldilocks::from_sha256_target(
                    finality_proof_public_inputs.validator_set_hash,
                    builder,
                ),
                authority_set_id: SingleTarget::from_u64_bits_le_lossy(
                    *finality_proof_public_inputs.message.authority_set_id,
                    builder,
                ),
                message_contents: MessageTargetGoldilocks::from_bit_array(message_targets, builder),
            }
        };

        composition_builder.build(targets_op)
    }
}
