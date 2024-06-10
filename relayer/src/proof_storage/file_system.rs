use std::{fs, path::PathBuf};

use super::{AuthoritySetId, InMemoryProofStorage, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

pub struct FileSystemProofStorage {
    cache: InMemoryProofStorage,
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

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        self.cache.get_latest_authority_set_id()
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        self.cache.get_proof_for_authority_set_id(authority_set_id)
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        self.cache.update(proof, new_authority_set_id)?;
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
            cache: InMemoryProofStorage::default(),
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
        let circuit_data = self
            .cache
            .circuit_data
            .clone()
            .ok_or(ProofStorageError::NotInitialized)?;

        fs::write(
            self.save_to.join("circuit_data.bin"),
            circuit_data.clone().into_bytes(),
        )
        .map_err(|_| ProofStorageError::NotInitialized)?;

        for (validator_set_id, proof) in &self.cache.proofs {
            fs::write(
                &self
                    .save_to
                    .join(&format!("proof_{}.bin", validator_set_id)),
                proof.clone().into_bytes(),
            )
            .map_err(|_| ProofStorageError::NotInitialized)?;
        }

        Ok(())
    }

    fn load_state(&mut self) -> Result<(), ProofStorageError> {
        let circuit_data = fs::read(self.save_to.join("circuit_data.bin"))
            .map_err(|_| ProofStorageError::NotInitialized)?;
        self.cache.circuit_data = Some(CircuitData::from_bytes(circuit_data));

        let prefix = "proof_";
        let postfix = ".bin";
        let found_validator_set_ids = fs::read_dir(&self.save_to)
            .map_err(|_| ProofStorageError::NotInitialized)?
            .filter_map(|file| {
                let file_name = file.expect("Failed to read file").file_name();
                let file_name = file_name.to_str();

                let valid_name = file_name
                    .map(|str| (&str[..prefix.len()], &str[str.len() - postfix.len()..]))
                    == Some((prefix, postfix));

                if valid_name {
                    let file_name = file_name.expect("Invalid file name");
                    let set_id = &file_name[prefix.len()..file_name.len() - postfix.len()];
                    let set_id: u64 = set_id.parse().expect("Invalid file name");
                    Some(set_id)
                } else {
                    None
                }
            });

        for validator_set_id in found_validator_set_ids {
            let proof = fs::read(
                &self
                    .save_to
                    .join(&format!("proof_{}.bin", validator_set_id)),
            )
            .map_err(|_| ProofStorageError::NotInitialized)?;

            self.cache
                .proofs
                .insert(validator_set_id, Proof::from_bytes(proof));
        }

        Ok(())
    }
}
