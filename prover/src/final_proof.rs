use itertools::Itertools;
use plonky2::plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig};
use plonky2_field::types::Field;

use crate::{
    common::{
        targets::{
            impl_target_set, MessageTargetGoldilocks, Sha256TargetGoldilocks, SingleTarget,
            TargetSet,
        },
        ProofComposition,
    },
    consts::{GENESIS_AUTHORITY_SET_ID, GENESIS_VALIDATOR_SET_HASH},
    latest_validator_set::LatestValidatorSetTarget,
    message_sent::{MessageSent, MessageSentTarget},
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct FinalProofTarget {
        message_contents: MessageTargetGoldilocks,
        block_number: SingleTarget
    }
}

pub struct FinalProof<const MESSAGE_LENGTH_IN_BITS: usize>
where
    [(); MESSAGE_LENGTH_IN_BITS / 8]:,
{
    // TODO: we can get rid of CircuitData and provie only the proof here as CircuitData is constant.
    pub current_validator_set: ProofWithCircuitData<LatestValidatorSetTarget>,
    pub message_sent: MessageSent<MESSAGE_LENGTH_IN_BITS>,
}

impl<const MESSAGE_LENGTH_IN_BITS: usize> FinalProof<MESSAGE_LENGTH_IN_BITS>
where
    [(); MESSAGE_LENGTH_IN_BITS / 8]:,
{
    pub fn prove(&self) -> ProofWithCircuitData<FinalProofTarget> {
        log::info!("Final proof...");

        let message_sent_proof = self.message_sent.prove();
        let desired_verifier_data = &self.current_validator_set.circuit_data().verifier_only;

        log::info!("Composing message sent and current validator set proofs...");

        let mut config = CircuitConfig::standard_recursion_config();
        config.fri_config.cap_height = 0;
        let composition_builder = ProofComposition::new_with_config(
            self.current_validator_set.clone(),
            message_sent_proof,
            config,
        );

        let targets_op = |builder: &mut CircuitBuilder<F, D>,
                          current_validator_set_proof: LatestValidatorSetTarget,
                          message_sent_proof: MessageSentTarget| {
            message_sent_proof
                .validator_set_hash
                .connect(&current_validator_set_proof.current_hash, builder);
            message_sent_proof
                .authority_set_id
                .connect(&current_validator_set_proof.current_set_id, builder);

            let desired_verifier_data = builder.constant_verifier_data(desired_verifier_data);
            desired_verifier_data.circuit_digest.connect(
                &current_validator_set_proof.verifier_data.circuit_digest,
                builder,
            );
            desired_verifier_data
                .constants_sigmas_cap
                .0
                .into_iter()
                .zip_eq(
                    current_validator_set_proof
                        .verifier_data
                        .merkle_caps
                        .into_iter(),
                )
                .for_each(|(hash_lhs, hash_rhs)| hash_lhs.connect(&hash_rhs, builder));

            let desired_genesis_authority_set_id =
                builder.constant(F::from_noncanonical_u64(GENESIS_AUTHORITY_SET_ID));
            builder.connect(
                desired_genesis_authority_set_id,
                current_validator_set_proof.genesis_set_id.to_target(),
            );

            let desired_genesis_validator_set_hash = Sha256TargetGoldilocks::parse_exact(
                &mut GENESIS_VALIDATOR_SET_HASH
                    .iter()
                    .map(|el| builder.constant(F::from_noncanonical_u64(*el))),
            );
            desired_genesis_validator_set_hash
                .connect(&current_validator_set_proof.genesis_hash, builder);

            // TODO: Replace with the actual block number.
            FinalProofTarget {
                message_contents: message_sent_proof.message_contents,
                block_number: message_sent_proof.authority_set_id,
            }
        };

        composition_builder
            .assert_both_circuit_digests()
            .compose(targets_op)
    }
}
