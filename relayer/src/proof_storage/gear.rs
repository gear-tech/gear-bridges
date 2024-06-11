use super::{in_memory::InMemoryProofStorage, AuthoritySetId, ProofStorage, ProofStorageError};
use gear_rpc_client::GearApi;
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

pub struct GearProofStorage {
    gear_api: GearApi,
    cache: InMemoryProofStorage,
}

impl ProofStorage for GearProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        let cached = self.cache.get_circuit_data();

        if cached.is_err() {
            // TODO: fetch from RPC.
        }

        cached
    }

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        todo!()
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let cached = self.cache.get_proof_for_authority_set_id(authority_set_id);

        if cached.is_err() {
            // TODO: fetch from RPC
        }

        cached
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }
}

impl GearProofStorage {
    pub async fn new(endpoint: &str) -> GearProofStorage {
        GearProofStorage {
            gear_api: GearApi::new(endpoint)
                .await
                .expect("Failed to create gear rpc client"),
            cache: Default::default(),
        }
    }
}
