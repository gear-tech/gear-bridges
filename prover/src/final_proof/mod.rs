//! ### Circuit that's used to ceate proof that will be submitted to ethereum.

use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, VerifierCircuitData},
        proof::ProofWithPublicInputs,
    },
};
use plonky2_field::types::Field;

use crate::{
    common::{
        targets::{impl_target_set, Blake2TargetGoldilocks, MessageTargetGoldilocks, TargetSet},
        BuilderExt, ProofWithCircuitData,
    },
    latest_validator_set::LatestValidatorSetTarget,
    prelude::*,
    proving::GenesisConfig,
};

pub mod message_sent;

use message_sent::MessageSent;

impl_target_set! {
    /// Public inputs for `FinalProof`.
    pub struct FinalProofTarget {
        /// Merkle trie root of queued messages.
        message_contents: MessageTargetGoldilocks,
        /// Block which contains merkle trie root of queued messages.
        block_number: Target
    }
}

pub struct FinalProof {
    /// Verifier data that will be the same for all the `LatestValidatorSet` proofs.
    pub current_validator_set_verifier_data: VerifierCircuitData<F, C, D>,
    /// Proof of `LatestValidatorSet` circuit.
    pub current_validator_set_proof: ProofWithPublicInputs<F, C, D>,
    /// Proof that message was queued for relaying.
    pub message_sent: MessageSent,
}

impl FinalProof {
    pub fn prove(self, genesis_config: GenesisConfig) -> ProofWithCircuitData<FinalProofTarget> {
        let message_sent_proof = self.message_sent.prove();

        log::debug!("Composing message sent and latest validator set proofs...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let message_sent_target =
            builder.recursively_verify_constant_proof(&message_sent_proof, &mut witness);

        let latest_validator_set_target = {
            let proof_with_pis_target = builder
                .add_virtual_proof_with_pis(&self.current_validator_set_verifier_data.common);
            let verifier_data_target = builder
                .constant_verifier_data(&self.current_validator_set_verifier_data.verifier_only);

            witness.set_proof_with_pis_target(
                &proof_with_pis_target,
                &self.current_validator_set_proof,
            );

            builder.verify_proof::<C>(
                &proof_with_pis_target,
                &verifier_data_target,
                &self.current_validator_set_verifier_data.common,
            );

            LatestValidatorSetTarget::parse_exact(
                &mut proof_with_pis_target.public_inputs.into_iter(),
            )
        };

        message_sent_target
            .validator_set_hash
            .connect(&latest_validator_set_target.current_hash, &mut builder);
        message_sent_target
            .authority_set_id
            .connect(&latest_validator_set_target.current_set_id, &mut builder);

        let desired_genesis_authority_set_id =
            builder.constant(F::from_noncanonical_u64(genesis_config.authority_set_id));
        builder.connect(
            desired_genesis_authority_set_id,
            latest_validator_set_target.genesis_set_id,
        );

        let desired_genesis_validator_set_hash = Blake2TargetGoldilocks::parse_exact(
            &mut genesis_config
                .validator_set_hash
                .iter()
                .map(|el| builder.constant(F::from_noncanonical_u64(*el))),
        );
        desired_genesis_validator_set_hash
            .connect(&latest_validator_set_target.genesis_hash, &mut builder);

        FinalProofTarget {
            message_contents: message_sent_target.message_contents,
            block_number: message_sent_target.block_number,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
