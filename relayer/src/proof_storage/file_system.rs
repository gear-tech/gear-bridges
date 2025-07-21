use super::{AuthoritySetId, InMemoryProofStorage, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};
use std::path::PathBuf;
use tokio::fs;

pub struct FileSystemProofStorage {
    cache: InMemoryProofStorage,
    save_to: PathBuf,
}

#[async_trait::async_trait]
impl ProofStorage for FileSystemProofStorage {
    async fn init(
        &self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        self.cache
            .init(proof_with_circuit_data, genesis_validator_set_id)
            .await?;
        self.save_state().await?;
        Ok(())
    }

    async fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        self.cache.get_circuit_data().await
    }

    async fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        self.cache.get_latest_authority_set_id().await
    }

    async fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        self.cache
            .get_proof_for_authority_set_id(authority_set_id)
            .await
    }

    async fn update(
        &self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        self.cache.update(proof, new_authority_set_id).await?;
        self.save_state().await?;
        Ok(())
    }
}

impl FileSystemProofStorage {
    pub async fn new(save_to: PathBuf) -> FileSystemProofStorage {
        fs::create_dir_all(&save_to)
            .await
            .expect("Failed to create directory for proof storage");
        if !save_to.is_dir() {
            panic!("Please provide directory as a path");
        }

        let mut storage = FileSystemProofStorage {
            cache: InMemoryProofStorage::default(),
            save_to,
        };

        if storage.load_state().await.is_ok() {
            log::info!("Proof storage state loaded successfully");
        } else {
            log::info!("Proof storage state not found. Waiting for initialization");
        }

        storage
    }

    async fn save_state(&self) -> Result<(), ProofStorageError> {
        let circuit_data = self.cache.get_circuit_data().await?;

        fs::write(
            self.save_to.join("circuit_data.bin"),
            circuit_data.clone().into_bytes(),
        )
        .await
        .map_err(|_| ProofStorageError::NotInitialized)?;

        let inner = self.cache.inner().read().await;

        for (validator_set_id, proof) in &inner.proofs {
            fs::write(
                self.save_to.join(format!("proof_{validator_set_id}.bin")),
                proof.clone().into_bytes(),
            )
            .await
            .map_err(|_| ProofStorageError::NotInitialized)?;
        }

        Ok(())
    }

    async fn load_state(&mut self) -> Result<(), ProofStorageError> {
        let circuit_data = fs::read(self.save_to.join("circuit_data.bin"))
            .await
            .map_err(|_| ProofStorageError::NotInitialized)?;
        self.cache.inner().write().await.circuit_data = Some(CircuitData::from_bytes(circuit_data));

        let prefix = "proof_";
        let postfix = ".bin";
        let mut read_dir = fs::read_dir(&self.save_to)
            .await
            .map_err(|_| ProofStorageError::NotInitialized)?;

        let mut found_validator_set_ids = Vec::new();

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|_| ProofStorageError::NotInitialized)?
        {
            let file_name = entry.file_name();
            let file_name = file_name.to_str();

            let valid_name = file_name
                .map(|str| (&str[..prefix.len()], &str[str.len() - postfix.len()..]))
                == Some((prefix, postfix));

            if valid_name {
                let file_name = file_name.expect("Invalid file name");
                let set_id = &file_name[prefix.len()..file_name.len() - postfix.len()];
                let set_id: u64 = set_id.parse().expect("Invalid file name");
                found_validator_set_ids.push(set_id);
            }
        }

        let mut inner = self.cache.inner().write().await;
        for validator_set_id in found_validator_set_ids {
            let proof = fs::read(self.save_to.join(format!("proof_{validator_set_id}.bin")))
                .await
                .map_err(|_| ProofStorageError::NotInitialized)?;

            inner
                .proofs
                .insert(validator_set_id, Proof::from_bytes(proof));
        }

        Ok(())
    }
}
