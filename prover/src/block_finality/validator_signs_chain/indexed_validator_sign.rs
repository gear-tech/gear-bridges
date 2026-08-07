//! ### Circuit that's used to prove that validator with particular index in validator set have
//! ### signed GRANDPA message.

use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitData},
        proof::ProofWithPublicInputsTarget,
    },
};

use plonky2_field::types::Field;
use std::sync::{Arc, OnceLock};

use super::{
    single_validator_sign::{PublicInputsTarget as SingleValidatorSignTarget, SingleValidatorSign},
    GrandpaMessageTarget,
};
use crate::{
    block_finality::validator_set_hash::ValidatorSetHashTarget,
    common::{
        targets::{impl_target_set, Blake2Target, TargetSet},
        ProofWithCircuitData,
    },
    consts::GRANDPA_VOTE_LENGTH,
    prelude::*,
};

impl_target_set! {
    /// Public inputs for `IndexedValidatorSign`.
    pub struct IndexedValidatorSignTarget {
        /// Blake2 hash of concatenated validator set public inputs.
        pub validator_set_hash: Blake2Target,
        /// Overall validator count in validator set.
        pub validator_count: Target,
        /// Validator index that have signed GRANDPA message.
        pub validator_idx: Target,
        /// GRANDPA message.
        pub message: GrandpaMessageTarget,
    }
}

pub struct IndexedValidatorSign {
    /// Public key corresponding to validator at specified index.
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    /// Index of validator that've signed the message.
    pub index: usize,
    /// GRANDPA message.
    pub message: [u8; GRANDPA_VOTE_LENGTH],
    /// Signature corresponding to validator at specified index.
    pub signature: [u8; consts::ED25519_SIGNATURE_SIZE],
}

struct IndexedValidatorSignCircuitTemplate {
    circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    validator_set_hash_proof: ProofWithPublicInputsTarget<D>,
    sign_proof: ProofWithPublicInputsTarget<D>,
    index_target: Target,
}

impl IndexedValidatorSignCircuitTemplate {
    fn cached(
        validator_set_hash_proof: &ProofWithCircuitData<ValidatorSetHashTarget>,
        sign_proof: &ProofWithCircuitData<SingleValidatorSignTarget>,
    ) -> &'static Self {
        // The recursive circuit shape is independent of the validator index,
        // signature, and message. Reuse it while supplying those proofs as
        // witnesses for each signer.
        static CACHE: OnceLock<IndexedValidatorSignCircuitTemplate> = OnceLock::new();
        CACHE.get_or_init(|| Self::build(validator_set_hash_proof, sign_proof))
    }

    fn build(
        validator_set_hash_proof: &ProofWithCircuitData<ValidatorSetHashTarget>,
        sign_proof: &ProofWithCircuitData<SingleValidatorSignTarget>,
    ) -> Self {
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());

        let validator_set_hash_data = validator_set_hash_proof.circuit_data();
        let validator_set_hash_proof_target =
            builder.add_virtual_proof_with_pis(&validator_set_hash_data.common);
        let validator_set_hash_verifier =
            builder.constant_verifier_data(&validator_set_hash_data.verifier_only);
        builder.verify_proof::<C>(
            &validator_set_hash_proof_target,
            &validator_set_hash_verifier,
            &validator_set_hash_data.common,
        );
        let validator_set_hash_target = ValidatorSetHashTarget::parse_exact(
            &mut validator_set_hash_proof_target
                .public_inputs
                .clone()
                .into_iter(),
        );

        let index_target = builder.add_virtual_target();
        let validator = validator_set_hash_target
            .validator_set
            .random_read(index_target, &mut builder);

        let sign_data = sign_proof.circuit_data();
        let sign_proof_target = builder.add_virtual_proof_with_pis(&sign_data.common);
        let sign_verifier = builder.constant_verifier_data(&sign_data.verifier_only);
        builder.verify_proof::<C>(&sign_proof_target, &sign_verifier, &sign_data.common);
        let sign_target = SingleValidatorSignTarget::parse_exact(
            &mut sign_proof_target.public_inputs.clone().into_iter(),
        );

        validator.connect(&sign_target.public_key, &mut builder);

        IndexedValidatorSignTarget {
            validator_set_hash: validator_set_hash_target.hash,
            validator_count: validator_set_hash_target.validator_set_length,
            validator_idx: index_target,
            message: sign_target.message,
        }
        .register_as_public_inputs(&mut builder);

        let circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(circuit_data.verifier_data());

        Self {
            circuit_data,
            verifier_data,
            validator_set_hash_proof: validator_set_hash_proof_target,
            sign_proof: sign_proof_target,
            index_target,
        }
    }
}

impl IndexedValidatorSign {
    pub fn prove(
        &self,
        valiadtor_set_hash_proof: &ProofWithCircuitData<ValidatorSetHashTarget>,
    ) -> ProofWithCircuitData<IndexedValidatorSignTarget> {
        log::debug!("    Proving indexed validator sign...");

        let sign_proof = SingleValidatorSign {
            public_key: self.public_key,
            signature: self.signature,
            message: self.message,
        }
        .prove();
        let template =
            IndexedValidatorSignCircuitTemplate::cached(valiadtor_set_hash_proof, &sign_proof);

        let mut witness = PartialWitness::new();
        witness.set_proof_with_pis_target(
            &template.validator_set_hash_proof,
            &valiadtor_set_hash_proof.proof(),
        );
        witness.set_proof_with_pis_target(&template.sign_proof, &sign_proof.proof());
        witness.set_target(template.index_target, F::from_canonical_usize(self.index));

        ProofWithCircuitData::prove_from_shared_circuit_data(
            &template.circuit_data,
            Arc::clone(&template.verifier_data),
            witness,
        )
    }
}
