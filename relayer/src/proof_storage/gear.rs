use futures::executor::block_on;
use gclient::{GearApi, WSAddress};

use super::{in_memory::InMemoryProofStorage, AuthoritySetId, ProofStorage, ProofStorageError};
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
        block_on(self.init_inner(proof_with_circuit_data, genesis_validator_set_id))
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        block_on(self.get_circuit_data_inner())
    }

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        block_on(self.get_latest_authority_set_id_inner())
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        block_on(self.get_proof_for_authority_set_id_inner(authority_set_id))
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        block_on(self.update_inner(proof, new_authority_set_id))
    }
}

impl GearProofStorage {
    pub async fn new(endpoint: &str, fee_payer: &str) -> anyhow::Result<GearProofStorage> {
        let endpoint: Vec<_> = endpoint.split(':').collect();
        let domain = endpoint[0];
        let port = endpoint[1].parse::<u16>()?;
        let address = WSAddress::try_new(domain, port)?;

        let gear_api = GearApi::init_with(address, fee_payer).await?;

        Ok(GearProofStorage {
            gear_api,
            cache: Default::default(),
        })
    }

    async fn init_inner(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        let (code_id, _) = self
            .gear_api
            .upload_code(gear_proof_storage::WASM_BINARY)
            .await
            .unwrap();

        let payload = gear_proof_storage::InitMessage {
            genesis_proof: gear_proof_storage::Proof {
                circuit_data: proof_with_circuit_data.circuit_data.0,
                proof: proof_with_circuit_data.proof.0,
                authority_set_id: genesis_validator_set_id + 1,
            },
        };

        //self.gear_api.create_program(code_id, &[], payload, 0, 0);

        todo!()
    }

    async fn get_circuit_data_inner(&self) -> Result<CircuitData, ProofStorageError> {
        let cached = self.cache.get_circuit_data();

        if cached.is_err() {
            // TODO: fetch from RPC.
        }

        cached
    }

    async fn get_latest_authority_set_id_inner(&self) -> Option<AuthoritySetId> {
        todo!()
    }

    async fn get_proof_for_authority_set_id_inner(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let cached = self.cache.get_proof_for_authority_set_id(authority_set_id);

        if cached.is_err() {
            // TODO: fetch from RPC
        }

        cached
    }

    async fn update_inner(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }
}
