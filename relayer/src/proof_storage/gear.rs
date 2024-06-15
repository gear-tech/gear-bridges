use std::collections::HashMap;

use futures::executor::block_on;
use gclient::{GearApi, WSAddress};
use gear_core::ids::ProgramId;
use parity_scale_codec::Encode;

use super::{AuthoritySetId, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

pub struct GearProofStorage {
    gear_api: GearApi,
    program: Option<ProgramId>,
    cache: Cache,
}

#[derive(Default)]
struct Cache {
    circuit_data: Option<CircuitData>,
    proofs: HashMap<u64, Proof>,
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
            program: None,
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
            .map_err(Into::<anyhow::Error>::into)?;

        let payload = gear_proof_storage::InitMessage {
            genesis_proof: gear_proof_storage::Proof {
                circuit_data: proof_with_circuit_data.circuit_data.0,
                proof: proof_with_circuit_data.proof.0,
                authority_set_id: genesis_validator_set_id + 1,
            },
        };

        let gas = self
            .gear_api
            .calculate_create_gas(None, code_id, payload.encode(), 0, false)
            .await
            .map_err(Into::<anyhow::Error>::into)?
            .min_limit;

        let (_, program, _) = self
            .gear_api
            .create_program(code_id, &[], payload, gas, 0)
            .await
            .map_err(Into::<anyhow::Error>::into)?;

        self.program = Some(program);

        Ok(())
    }

    async fn get_circuit_data_inner(&self) -> Result<CircuitData, ProofStorageError> {
        if let Some(circuit_data) = self.cache.circuit_data.as_ref() {
            return Ok(circuit_data.clone());
        }

        // TODO: Fetch from gear and add to cache

        todo!()
    }

    async fn get_latest_authority_set_id_inner(&self) -> Option<AuthoritySetId> {
        self.read_state()
            .await
            .ok()
            .map(|s| s.latest_proof.authority_set_id)
    }

    async fn get_proof_for_authority_set_id_inner(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let circuit_data = self.get_circuit_data_inner().await?;

        if let Some(proof) = self.cache.proofs.get(&authority_set_id) {
            return Ok(ProofWithCircuitData {
                circuit_data,
                proof: proof.clone(),
            });
        }

        // TODO: Fetch from gear

        todo!()
    }

    async fn update_inner(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        let _ = self.cache.proofs.insert(new_authority_set_id, proof);

        // TODO: Submit to gear.

        todo!()
    }

    async fn read_state(&self) -> Result<gear_proof_storage::State, ProofStorageError> {
        let Some(program) = self.program else {
            return Err(ProofStorageError::NotInitialized);
        };

        let state: gear_proof_storage::State = self
            .gear_api
            .read_state(program, vec![])
            .await
            .map_err(Into::<anyhow::Error>::into)?;

        Ok(state)
    }
}
