use std::sync::Arc;

use gear_rpc_client::GearApi;
use primitive_types::H256;
use prover::proving::GenesisConfig;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

/// A separate thread responsible for running block finality prover.
pub struct FinalityProver {
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
}

impl FinalityProver {
    pub fn new(
        api_provider: ApiProviderConnection,
        proof_storage: Arc<dyn ProofStorage>,
        genesis_config: GenesisConfig,
    ) -> Self {
        Self {
            api_provider,
            proof_storage,
            genesis_config,
        }
    }

    async fn process(
        &mut self,
        requests: &mut UnboundedReceiver<Request>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.client();
        while let Some(request) = requests.recv().await {
            let proof = self
                .generate_proof(
                    &gear_api,
                    request.block_number,
                    request.block_hash,
                    request.merkle_root,
                )
                .await?;

            if responses
                .send(Response {
                    block_number: request.block_number,
                    merkle_root: request.merkle_root,
                    proof,
                })
                .is_err()
            {
                log::warn!("Proof response channel closed, exiting");
                break;
            }
        }

        Ok(())
    }

    async fn generate_proof(
        &mut self,
        gear_api: &GearApi,
        block_number: u32,
        block_hash: H256,
        merkle_root: H256,
    ) -> anyhow::Result<FinalProof> {
        log::info!("Generating merkle root proof for block #{block_number}");

        log::info!("Proving merkle root({merkle_root}:?) presence in block #{block_number}");

        let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

        let inner_proof = self
            .proof_storage
            .get_proof_for_authority_set_id(authority_set_id)
            .await?;

        let proof =
            prover_interface::prove_final(&gear_api, inner_proof, self.genesis_config, block_hash)
                .await?;

        Ok(proof)
    }
}

pub struct Request {
    pub block_number: u32,
    pub block_hash: H256,
    pub merkle_root: H256,
}

pub struct Response {
    pub block_number: u32,
    pub merkle_root: H256,
    pub proof: FinalProof,
}
