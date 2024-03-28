use std::{fs, io, path::PathBuf};

use plonky2::{
    plonk::{
        circuit_data::{CommonCircuitData, VerifierCircuitData, VerifierOnlyCircuitData},
        proof::ProofWithPublicInputs,
    },
    util::serialization::DefaultGateSerializer,
};
use prover::{
    common::targets::ParsableTargetSet, latest_validator_set::LatestValidatorSetTarget, prelude::*,
};

#[derive(Debug)]
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

    fn get_verifier_circuit_data(&self) -> Result<VerifierCircuitData<F, C, D>, ProofStorageError>;

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>>;

    // TODO: Add fn to query any of the stored proofs

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError>;
}

#[derive(Default)]
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

    fn get_verifier_circuit_data(&self) -> Result<VerifierCircuitData<F, C, D>, ProofStorageError> {
        self.verifier_circuit_data
            .clone()
            .ok_or(ProofStorageError::NotInitialized)
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

pub struct PersistentMockProofStorage {
    mock: MockProofStorage,
    save_to: PathBuf,
}

impl PersistentMockProofStorage {
    pub fn new(save_to: PathBuf) -> PersistentMockProofStorage {
        fs::create_dir_all(&save_to).unwrap();
        if !save_to.is_dir() {
            panic!("Please provide directory as a path");
        }

        let mut storage = PersistentMockProofStorage {
            mock: MockProofStorage::default(),
            save_to,
        };

        storage.load_state();

        storage
    }

    fn save_state(&self) {
        let verifier_circuit_data = self.mock.verifier_circuit_data.as_ref().unwrap();
        let proof_with_public_inputs = self.mock.latest_proof.as_ref().unwrap();

        let common = verifier_circuit_data
            .common
            .to_bytes(&DefaultGateSerializer {})
            .unwrap();
        let verifier_only = verifier_circuit_data.verifier_only.to_bytes().unwrap();
        let proof = proof_with_public_inputs.to_bytes();

        fs::write(&self.save_to.join("common_circuit_data.bin"), common).unwrap();
        fs::write(
            &self.save_to.join("verifier_only_circuit_data.bin"),
            verifier_only,
        )
        .unwrap();
        fs::write(&self.save_to.join("proof_with_public_inputs.bin"), proof).unwrap();
    }

    fn load_state(&mut self) {
        let mut load = || -> io::Result<()> {
            let common_data = fs::read(&self.save_to.join("common_circuit_data.bin"))?;
            let verifier_only_data =
                fs::read(&self.save_to.join("verifier_only_circuit_data.bin"))?;
            let proof_data = fs::read(&self.save_to.join("proof_with_public_inputs.bin"))?;

            let common =
                CommonCircuitData::<F, D>::from_bytes(common_data, &DefaultGateSerializer {})
                    .unwrap();
            let verifier_only =
                VerifierOnlyCircuitData::<C, D>::from_bytes(verifier_only_data).unwrap();
            let proof = ProofWithPublicInputs::<F, C, D>::from_bytes(proof_data, &common).unwrap();

            self.mock.latest_proof = Some(proof);
            self.mock.verifier_circuit_data = Some(VerifierCircuitData {
                verifier_only,
                common,
            });

            Ok(())
        };

        let _ = load();
    }
}

impl ProofStorage for PersistentMockProofStorage {
    fn init(
        &mut self,
        verifier_circuit_data: VerifierCircuitData<F, C, D>,
        proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
    ) -> Result<(), ProofStorageError> {
        self.mock
            .init(verifier_circuit_data, proof_with_public_inputs)?;
        self.save_state();
        Ok(())
    }

    fn get_verifier_circuit_data(&self) -> Result<VerifierCircuitData<F, C, D>, ProofStorageError> {
        self.mock.get_verifier_circuit_data()
    }

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>> {
        self.mock.get_latest_proof()
    }

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError> {
        self.mock.update(proof)?;
        self.save_state();
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

    fn get_verifier_circuit_data(&self) -> Result<VerifierCircuitData<F, C, D>, ProofStorageError> {
        todo!()
    }

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>> {
        todo!()
    }

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError> {
        todo!()
    }
}
