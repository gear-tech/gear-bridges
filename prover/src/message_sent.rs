use plonky2::{
    fri::FriConfig,
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use crate::{
    block_finality::{BlockFinality, BlockFinalityTarget},
    common::{
        targets::{BitArrayTarget, Sha256Target, TargetSetOperations},
        ProofCompositionBuilder, ProofCompositionTargets, TargetSet,
    },
    merkle_proof::{MerkleProof, MerkleProofTarget},
    prelude::*,
    ProofWithCircuitData,
};

#[derive(Clone)]
pub struct MessageSentTarget<const MESSAGE_LENGTH_IN_BITS: usize> {
    validator_set_hash: Sha256Target,
    message_contents: BitArrayTarget<MESSAGE_LENGTH_IN_BITS>,
}

impl<const MESSAGE_LENGTH_IN_BITS: usize> TargetSet for MessageSentTarget<MESSAGE_LENGTH_IN_BITS>
where
    [(); MESSAGE_LENGTH_IN_BITS / 8]:,
{
    fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
        Self {
            validator_set_hash: Sha256Target::parse(raw),
            message_contents: BitArrayTarget::parse(raw),
        }
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
    pub fn prove(&self) -> ProofWithCircuitData<MessageSentTarget<MESSAGE_LENGTH_IN_BITS>> {
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

            finality_proof_public_inputs
                .validator_set_hash
                .register_as_public_inputs(builder);

            inclusion_proof_public_inputs
                .leaf_data
                .register_as_public_inputs(builder);

            inclusion_proof_public_inputs
                .root_hash
                .connect(&finality_proof_public_inputs.message.block_hash, builder);
        };

        composition_builder
            .operation_with_targets(targets_op)
            .build()
    }
}
