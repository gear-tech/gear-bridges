use std::collections::BTreeMap;

use super::{AuthoritySetId, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

#[derive(Default)]
pub struct InMemoryProofStorage {
    pub(super) proofs: BTreeMap<AuthoritySetId, Proof>,
    pub(super) circuit_data: Option<CircuitData>,
}

impl ProofStorage for InMemoryProofStorage {
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

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        self.proofs.last_key_value().map(|(k, _)| *k)
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let circuit_data = self.get_circuit_data()?;

        let proof = self
            .proofs
            .get(&authority_set_id)
            .ok_or(ProofStorageError::NotFound(authority_set_id))?
            .clone();

        Ok(ProofWithCircuitData {
            proof,
            circuit_data,
        })
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        let authority_set_id = self
            .proofs
            .last_key_value()
            .map(|(k, _)| *k)
            .expect("Proof storage not initialized");

        if new_authority_set_id != authority_set_id + 1 {
            return Err(ProofStorageError::AuthoritySetIdMismatch);
        }

        if self.proofs.insert(authority_set_id + 1, proof).is_some() {
            panic!(
                "Proof for validator set id = {} already present",
                authority_set_id + 1
            )
        }

        Ok(())
    }
}
