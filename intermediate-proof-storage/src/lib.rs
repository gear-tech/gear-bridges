use plonky2::{
    field::extension::Extendable,
    hash::hash_types::RichField,
    plonk::{
        circuit_data::{CommonCircuitData, VerifierCircuitData, VerifierOnlyCircuitData},
        config::GenericConfig,
        proof::ProofWithPublicInputs,
    },
};
use prover::{
    common::targets::ParsableTargetSet, latest_validator_set::LatestValidatorSetTarget, prelude::*,
};

pub enum ProofStorageError {
    AlreadyInitialized,
    NotInitialized,
    InvalidProof(anyhow::Error),
    InvalidCircuitData,
    InvalidValidatorSetId,
    InvalidGenesis,
    InvalidVerifierData,
}

pub trait ProofStorage {
    fn init(
        &mut self,
        verifier_circuit_data: VerifierCircuitData<F, C, D>,
        proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
    ) -> Result<(), ProofStorageError>;

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>>;

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError>;
}

struct MockProofStorage {
    latest_proof: Option<ProofWithPublicInputs<F, C, D>>,
    verifier_circuit_data: Option<VerifierCircuitData<F, C, D>>,
}

impl ProofStorage for MockProofStorage {
    fn init(
        &mut self,
        verifier_circuit_data: VerifierCircuitData<F, C, D>,
        proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
    ) -> Result<(), ProofStorageError> {
        if self.verifier_circuit_data.is_some() {
            return Err(ProofStorageError::AlreadyInitialized);
        }

        let verified = verifier_circuit_data.verify(proof_with_public_inputs.clone());
        if let Err(err) = verified {
            return Err(ProofStorageError::InvalidProof(err));
        }

        let public_inputs = LatestValidatorSetTarget::parse_public_inputs(
            &mut proof_with_public_inputs.public_inputs.clone().into_iter(),
        );

        if public_inputs.verifier_only_data != verifier_circuit_data.verifier_only {
            return Err(ProofStorageError::InvalidVerifierData);
        }

        self.verifier_circuit_data = Some(verifier_circuit_data);
        self.latest_proof = Some(proof_with_public_inputs);

        Ok(())
    }

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>> {
        self.latest_proof.clone()
    }

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError> {
        let mut public_inputs = proof.public_inputs.clone().into_iter();
        let public_inputs = LatestValidatorSetTarget::parse_public_inputs(&mut public_inputs);

        if self.verifier_circuit_data.is_none() || self.latest_proof.is_none() {
            return Err(ProofStorageError::NotInitialized);
        }

        let verifier_data = self.verifier_circuit_data.as_ref().unwrap();
        let latest_proof = self.latest_proof.as_ref().unwrap();

        if let Err(err) = verifier_data.verify(proof.clone()) {
            return Err(ProofStorageError::InvalidProof(err));
        }

        let latest_proof_public_inputs = LatestValidatorSetTarget::parse_public_inputs(
            &mut latest_proof.public_inputs.clone().into_iter(),
        );

        if latest_proof_public_inputs.current_set_id + 1 != public_inputs.current_set_id {
            return Err(ProofStorageError::InvalidValidatorSetId);
        }

        if latest_proof_public_inputs.genesis_hash != public_inputs.genesis_hash
            || latest_proof_public_inputs.genesis_set_id != public_inputs.genesis_set_id
        {
            return Err(ProofStorageError::InvalidGenesis);
        }

        if latest_proof_public_inputs.verifier_only_data != public_inputs.verifier_only_data {
            return Err(ProofStorageError::InvalidVerifierData);
        }

        self.latest_proof = Some(proof);

        Ok(())
    }
}

struct VaraProofStorage {}

impl ProofStorage for VaraProofStorage {
    fn init(
        &mut self,
        verifier_circuit_data: VerifierCircuitData<F, C, D>,
        proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>> {
        todo!()
    }

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError> {
        todo!()
    }
}
