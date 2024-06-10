use super::{AuthoritySetId, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

#[derive(Default)]
pub struct GearProofStorage {}

impl ProofStorage for GearProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        todo!()
    }

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        todo!()
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        todo!()
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }
}
