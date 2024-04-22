use std::{fs, io, path::PathBuf};

use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

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
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError>;

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError>;

    fn get_latest_proof(&self) -> Option<Proof>;

    // TODO: Add fn to query any of the stored proofs

    fn update(&mut self, proof: Proof) -> Result<(), ProofStorageError>;
}

#[derive(Default)]
struct MockProofStorage {
    latest: Option<ProofWithCircuitData>,
    latest_validator_set_id: u64,
}

impl ProofStorage for MockProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        if self.latest.is_some() {
            return Err(ProofStorageError::AlreadyInitialized);
        }

        self.latest = Some(proof_with_circuit_data);
        self.latest_validator_set_id = genesis_validator_set_id;

        Ok(())
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        self.latest
            .as_ref()
            .map(|cd| cd.circuit_data.clone())
            .ok_or(ProofStorageError::NotInitialized)
    }

    fn get_latest_proof(&self) -> Option<Proof> {
        self.latest.as_ref().map(|cd| cd.proof.clone())
    }

    fn update(&mut self, proof: Proof) -> Result<(), ProofStorageError> {
        let circuit_data = self
            .latest
            .as_ref()
            .map(|cd| cd.circuit_data.clone())
            .ok_or_else(|| ProofStorageError::NotInitialized)?;

        self.latest = Some(ProofWithCircuitData {
            proof,
            circuit_data,
        });
        self.latest_validator_set_id += 1;

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
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        self.cache
            .init(proof_with_circuit_data, genesis_validator_set_id)?;
        self.save_state()?;
        Ok(())
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        self.cache.get_circuit_data()
    }

    fn get_latest_proof(&self) -> Option<Proof> {
        self.cache.get_latest_proof()
    }

    fn update(&mut self, proof: Proof) -> Result<(), ProofStorageError> {
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
        let proof_with_circuit_data = self
            .cache
            .latest
            .clone()
            .ok_or_else(|| ProofStorageError::NotInitialized)?;

        let write_files = || -> Result<(), io::Error> {
            fs::write(
                &self.save_to.join("circuit_data.bin"),
                proof_with_circuit_data.circuit_data.into_bytes(),
            )?;
            fs::write(
                &self.save_to.join("proof.bin"),
                proof_with_circuit_data.proof.into_bytes(),
            )?;
            Ok(())
        };

        write_files().map_err(|_| ProofStorageError::NotInitialized)
    }

    fn load_state(&mut self) -> Result<(), ProofStorageError> {
        let mut load = || -> io::Result<()> {
            let circuit_data = fs::read(&self.save_to.join("circuit_data.bin"))?;
            let proof = fs::read(&self.save_to.join("proof.bin"))?;

            self.cache.latest = Some(ProofWithCircuitData {
                proof: Proof::from_bytes(proof),
                circuit_data: CircuitData::from_bytes(circuit_data),
            });

            Ok(())
        };

        load().map_err(|_| ProofStorageError::NotInitialized)
    }
}
