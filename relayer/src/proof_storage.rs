use std::{collections::BTreeMap, fs, path::PathBuf};

use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

#[derive(Debug)]
pub enum ProofStorageError {
    AlreadyInitialized,
    NotInitialized,
}

pub trait ProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError>;

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError>;

    fn get_latest_proof(&self) -> Option<(ProofWithCircuitData, u64)>;

    // TODO: Add fn to query any of the stored proofs

    fn update(&mut self, proof: Proof) -> Result<(), ProofStorageError>;
}

type ValidatorSetId = u64;

#[derive(Default)]
struct MockProofStorage {
    proofs: BTreeMap<ValidatorSetId, Proof>,
    circuit_data: Option<CircuitData>,
}

impl ProofStorage for MockProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        if !self.proofs.is_empty() {
            return Err(ProofStorageError::AlreadyInitialized);
        }

        self.circuit_data = Some(proof_with_circuit_data.circuit_data);
        self.proofs
            .insert(genesis_validator_set_id + 1, proof_with_circuit_data.proof);

        Ok(())
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        self.circuit_data
            .clone()
            .ok_or(ProofStorageError::NotInitialized)
    }

    fn get_latest_proof(&self) -> Option<(ProofWithCircuitData, ValidatorSetId)> {
        self.proofs.last_key_value().map(|(k, v)| {
            (
                ProofWithCircuitData {
                    proof: v.clone(),
                    circuit_data: self
                        .circuit_data
                        .clone()
                        .expect("Proof storage not initialized"),
                },
                *k,
            )
        })
    }

    fn update(&mut self, proof: Proof) -> Result<(), ProofStorageError> {
        let validator_set_id = self
            .proofs
            .last_key_value()
            .map(|(k, _)| *k)
            .expect("Proof storage not initialized");

        self.proofs
            .insert(validator_set_id + 1, proof)
            .expect(&format!(
                "Proof for validator set id = {} already present",
                validator_set_id + 1
            ));

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

    fn get_latest_proof(&self) -> Option<(ProofWithCircuitData, u64)> {
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
        let circuit_data = self
            .cache
            .circuit_data
            .clone()
            .ok_or_else(|| ProofStorageError::NotInitialized)?;

        fs::write(
            &self.save_to.join("circuit_data.bin"),
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
        let circuit_data = fs::read(&self.save_to.join("circuit_data.bin"))
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
                .insert(validator_set_id, Proof::from_bytes(proof))
                .expect("Files with the same name detected");
        }

        Ok(())
    }
}
