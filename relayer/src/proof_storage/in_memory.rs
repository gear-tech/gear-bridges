use std::collections::BTreeMap;

use super::{AuthoritySetId, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};
use tokio::sync::RwLock;

#[derive(Default)]
pub struct InMemoryProofStorage {
    inner: RwLock<InMemoryProofStorageInner>,
}

impl InMemoryProofStorage {
    pub fn inner(&self) -> &RwLock<InMemoryProofStorageInner> {
        &self.inner
    }
}

#[derive(Default)]
pub struct InMemoryProofStorageInner {
    pub(super) proofs: BTreeMap<AuthoritySetId, Proof>,
    pub(super) circuit_data: Option<CircuitData>,
}

#[async_trait::async_trait]
impl ProofStorage for InMemoryProofStorage {
    async fn init(
        &self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        let mut inner = self.inner.write().await;
        if !inner.proofs.is_empty() {
            return Err(ProofStorageError::AlreadyInitialized);
        }

        inner.circuit_data = Some(proof_with_circuit_data.circuit_data);
        inner
            .proofs
            .insert(genesis_validator_set_id + 1, proof_with_circuit_data.proof);

        Ok(())
    }

    async fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        let inner = self.inner.read().await;
        inner
            .circuit_data
            .clone()
            .ok_or(ProofStorageError::NotInitialized)
    }

    async fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        let inner = self.inner.read().await;
        inner.proofs.last_key_value().map(|(k, _)| *k)
    }

    async fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let circuit_data = self.get_circuit_data().await?;
        let inner = self.inner.read().await;
        let proof = inner
            .proofs
            .get(&authority_set_id)
            .ok_or(ProofStorageError::NotFound(authority_set_id))?
            .clone();

        Ok(ProofWithCircuitData {
            proof,
            circuit_data,
        })
    }

    async fn update(
        &self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        let mut inner = self.inner.write().await;
        let authority_set_id = inner
            .proofs
            .last_key_value()
            .map(|(k, _)| *k)
            .expect("Proof storage not initialized");

        if new_authority_set_id != authority_set_id + 1 {
            return Err(ProofStorageError::AuthoritySetIdMismatch);
        }

        if inner.proofs.insert(authority_set_id + 1, proof).is_some() {
            panic!(
                "Proof for validator set id = {} already present",
                authority_set_id + 1
            )
        }

        Ok(())
    }
}
