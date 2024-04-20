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

        let (verifier_data, latest_proof) = match (
            self.verifier_circuit_data.as_ref(),
            self.latest_proof.as_ref(),
        ) {
            (Some(verifier_data), Some(latest_proof)) => (verifier_data, latest_proof),
            _ => return Err(ProofStorageError::NotInitialized),
        };

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

pub struct FileSystemProofStorage {
    cache: MockProofStorage,
    save_to: PathBuf,
}

impl ProofStorage for FileSystemProofStorage {
    fn init(
        &mut self,
        verifier_circuit_data: VerifierCircuitData<F, C, D>,
        proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
    ) -> Result<(), ProofStorageError> {
        self.cache
            .init(verifier_circuit_data, proof_with_public_inputs)?;
        self.save_state()?;
        Ok(())
    }

    fn get_verifier_circuit_data(&self) -> Result<VerifierCircuitData<F, C, D>, ProofStorageError> {
        self.cache.get_verifier_circuit_data()
    }

    fn get_latest_proof(&self) -> Option<ProofWithPublicInputs<F, C, D>> {
        self.cache.get_latest_proof()
    }

    fn update(&mut self, proof: ProofWithPublicInputs<F, C, D>) -> Result<(), ProofStorageError> {
        self.cache.update(proof)?;
        self.save_state()?;
        Ok(())
    }
}

impl FileSystemProofStorage {
    pub fn new(save_to: PathBuf) -> FileSystemProofStorage {
        fs::create_dir_all(&save_to).expect("Failed to create directory for proof storage");
        if !save_to.is_dir() {
            panic!("Please provide directory as a path");
        }

        let mut storage = FileSystemProofStorage {
            cache: MockProofStorage::default(),
            save_to,
        };

        if storage.load_state().is_ok() {
            log::info!("Proof storage state loaded succesfully");
        } else {
            log::info!("Proof storage state not found. Waiting for initialization");
        }

        storage
    }

    fn save_state(&self) -> Result<(), ProofStorageError> {
        let (verifier_data, latest_proof) = match (
            self.cache.verifier_circuit_data.as_ref(),
            self.cache.latest_proof.as_ref(),
        ) {
            (Some(verifier_data), Some(latest_proof)) => (verifier_data, latest_proof),
            _ => return Err(ProofStorageError::NotInitialized),
        };

        let common = verifier_data
            .common
            .to_bytes(&DefaultGateSerializer {})
            .expect("Failed to serialize CommonCircuitData");
        let verifier_only = verifier_data
            .verifier_only
            .to_bytes()
            .expect("Failed to serialize VerifierOnlyCircuitData");
        let proof = latest_proof.to_bytes();

        let write_files = || -> Result<(), io::Error> {
            fs::write(&self.save_to.join("common_circuit_data.bin"), common)?;
            fs::write(
                &self.save_to.join("verifier_only_circuit_data.bin"),
                verifier_only,
            )?;
            fs::write(&self.save_to.join("proof_with_public_inputs.bin"), proof)?;
            Ok(())
        };

        write_files().map_err(|_| ProofStorageError::NotInitialized)
    }

    fn load_state(&mut self) -> Result<(), ProofStorageError> {
        let mut load = || -> io::Result<()> {
            let common_data = fs::read(&self.save_to.join("common_circuit_data.bin"))?;
            let verifier_only_data =
                fs::read(&self.save_to.join("verifier_only_circuit_data.bin"))?;
            let proof_data = fs::read(&self.save_to.join("proof_with_public_inputs.bin"))?;

            let common =
                CommonCircuitData::<F, D>::from_bytes(common_data, &DefaultGateSerializer {})
                    .expect("Failed to deserialize CommonCircuitData");
            let verifier_only = VerifierOnlyCircuitData::<C, D>::from_bytes(verifier_only_data)
                .expect("Failed to deserialize VerifierOnlyCircuitData");
            let proof = ProofWithPublicInputs::<F, C, D>::from_bytes(proof_data, &common)
                .expect("Failed to deserialize ProofWithPublicInputs");

            self.cache.latest_proof = Some(proof);
            self.cache.verifier_circuit_data = Some(VerifierCircuitData {
                verifier_only,
                common,
            });

            Ok(())
        };

        load().map_err(|_| ProofStorageError::NotInitialized)
    }
}
