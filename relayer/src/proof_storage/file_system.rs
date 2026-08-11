use super::{AuthoritySetId, InMemoryProofStorage, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::fs;

pub struct FileSystemProofStorage {
    cache: InMemoryProofStorage,
    save_to: PathBuf,
}

fn io_error(operation: &str, path: &Path, err: io::Error) -> ProofStorageError {
    ProofStorageError::InnerError(
        anyhow::Error::new(err).context(format!("{operation}: {}", path.display())),
    )
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
    pub async fn new(save_to: PathBuf) -> Result<FileSystemProofStorage, ProofStorageError> {
        fs::create_dir_all(&save_to)
            .await
            .map_err(|err| io_error("create proof storage directory", &save_to, err))?;

        let mut storage = FileSystemProofStorage {
            cache: InMemoryProofStorage::default(),
            save_to,
        };

        match storage.load_state().await {
            Ok(()) => log::info!("Proof storage state loaded successfully"),
            Err(ProofStorageError::NotInitialized) => {
                log::info!("Proof storage state not found. Waiting for initialization")
            }
            Err(err) => return Err(err),
        }

        Ok(storage)
    }

    async fn save_state(&self) -> Result<(), ProofStorageError> {
        let circuit_data = self.cache.get_circuit_data().await?;

        self.atomic_write("circuit_data.bin", circuit_data.clone().into_bytes())
            .await?;

        let inner = self.cache.inner().read().await;

        for (validator_set_id, proof) in &inner.proofs {
            self.atomic_write(
                &format!("proof_{validator_set_id}.bin"),
                proof.clone().into_bytes(),
            )
            .await?;
        }

        Ok(())
    }

    async fn atomic_write(&self, name: &str, bytes: Vec<u8>) -> Result<(), ProofStorageError> {
        let path = self.save_to.join(name);
        let tmp_path = self.save_to.join(format!("{name}.tmp"));
        fs::write(&tmp_path, bytes)
            .await
            .map_err(|err| io_error("write proof storage temporary file", &tmp_path, err))?;
        fs::rename(&tmp_path, &path)
            .await
            .map_err(|err| io_error("replace proof storage file", &path, err))?;
        Ok(())
    }

    async fn load_state(&mut self) -> Result<(), ProofStorageError> {
        let circuit_data_path = self.save_to.join("circuit_data.bin");
        let circuit_data = match fs::read(&circuit_data_path).await {
            Ok(circuit_data) => circuit_data,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(ProofStorageError::NotInitialized);
            }
            Err(err) => {
                return Err(io_error(
                    "read proof storage circuit data",
                    &circuit_data_path,
                    err,
                ));
            }
        };
        self.cache.inner().write().await.circuit_data = Some(CircuitData::from_bytes(circuit_data));

        let prefix = "proof_";
        let postfix = ".bin";
        let mut read_dir = fs::read_dir(&self.save_to)
            .await
            .map_err(|err| io_error("read proof storage directory", &self.save_to, err))?;

        let mut found_validator_set_ids = Vec::new();

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|err| io_error("read proof storage directory entry", &self.save_to, err))?
        {
            let file_name = entry.file_name();
            let Some(set_id) = file_name
                .to_str()
                .and_then(|name| name.strip_prefix(prefix))
                .and_then(|name| name.strip_suffix(postfix))
            else {
                continue;
            };
            let set_id = set_id.parse::<u64>().map_err(|err| {
                ProofStorageError::InnerError(anyhow::anyhow!(
                    "invalid authority-set proof file name {}: {err}",
                    entry.path().display()
                ))
            })?;
            found_validator_set_ids.push(set_id);
        }

        let mut inner = self.cache.inner().write().await;
        for validator_set_id in found_validator_set_ids {
            let proof_path = self.save_to.join(format!("proof_{validator_set_id}.bin"));
            let proof = fs::read(&proof_path)
                .await
                .map_err(|err| io_error("read authority-set proof", &proof_path, err))?;

            inner
                .proofs
                .insert(validator_set_id, Proof::from_bytes(proof));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_storage_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gear-bridges-{test_name}-{}", uuid::Uuid::new_v4()))
    }

    fn assert_inner_io_error(err: ProofStorageError) {
        match err {
            ProofStorageError::InnerError(err) => {
                assert!(err.chain().any(|cause| cause.is::<io::Error>()));
            }
            err => panic!("expected filesystem error, got {err}"),
        }
    }

    #[tokio::test]
    async fn missing_state_is_treated_as_uninitialized() {
        let path = temporary_storage_path("missing-proof-state");
        let storage = FileSystemProofStorage::new(path.clone()).await.unwrap();

        assert_eq!(storage.get_latest_authority_set_id().await, None);

        fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn storage_path_that_is_a_file_returns_io_error() {
        let path = temporary_storage_path("storage-path-is-file");
        fs::write(&path, b"not a directory").await.unwrap();

        let err = FileSystemProofStorage::new(path.clone())
            .await
            .err()
            .expect("a regular file cannot be used as a storage directory");
        assert_inner_io_error(err);

        fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn unreadable_circuit_data_is_reported_as_io_error() {
        let path = temporary_storage_path("invalid-circuit-data");
        fs::create_dir_all(path.join("circuit_data.bin"))
            .await
            .unwrap();

        let err = FileSystemProofStorage::new(path.clone())
            .await
            .err()
            .expect("a directory cannot be read as circuit data");
        assert_inner_io_error(err);

        fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_write_preserves_the_io_error() {
        let path = temporary_storage_path("failed-atomic-write");
        let storage = FileSystemProofStorage::new(path.clone()).await.unwrap();
        fs::create_dir(path.join("circuit_data.bin.tmp"))
            .await
            .unwrap();

        let err = storage
            .atomic_write("circuit_data.bin", vec![1, 2, 3])
            .await
            .unwrap_err();
        assert_inner_io_error(err);

        fs::remove_dir_all(path).await.unwrap();
    }
}
