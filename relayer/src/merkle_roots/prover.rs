use crate::{
    message_relayer::eth_to_gear::api_provider::ApiProviderConnection,
    prover_interface::{self, FinalProof},
};
use futures::executor::block_on;
use gear_rpc_client::GearApi;
use primitive_types::H256;
use prover::proving::{GenesisConfig, ProofWithCircuitData};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct FinalityProverIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl FinalityProverIo {
    pub fn new(requests: UnboundedSender<Request>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
            responses,
        }
    }

    pub fn prove(
        &mut self,
        block_number: u32,
        block_hash: H256,
        merkle_root: H256,
        inner_proof: ProofWithCircuitData,
    ) -> bool {
        self.requests
            .send(Request {
                block_number,
                block_hash,
                merkle_root,
                inner_proof,
            })
            .is_ok()
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

/// A separate thread responsible for running block finality prover.
pub struct FinalityProver {
    api_provider: ApiProviderConnection,
    genesis_config: GenesisConfig,
}

impl FinalityProver {
    pub fn new(api_provider: ApiProviderConnection, genesis_config: GenesisConfig) -> Self {
        Self {
            api_provider,

            genesis_config,
        }
    }

    pub fn run(mut self) -> FinalityProverIo {
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (res_tx, res_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = FinalityProverIo::new(req_tx, res_rx);

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                if let Err(e) = self.process(&mut req_rx, &res_tx).await {
                    log::error!("Error processing finality prover requests: {e}");
                } else {
                    log::info!("Prover exiting");
                }
            })
        });

        io
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
                    request.inner_proof,
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
        inner_proof: ProofWithCircuitData,
    ) -> anyhow::Result<FinalProof> {
        log::info!("Generating merkle root proof for block #{block_number}");

        log::info!("Proving merkle root({merkle_root}) presence in block #{block_number}");

        let proof =
            prover_interface::prove_final(gear_api, inner_proof, self.genesis_config, block_hash)
                .await?;
        log::info!("Proof for {merkle_root} generated (block #{block_number})");
        Ok(proof)
    }
}

pub struct Request {
    pub block_number: u32,
    pub block_hash: H256,
    pub merkle_root: H256,
    pub inner_proof: ProofWithCircuitData,
}

pub struct Response {
    pub block_number: u32,
    pub merkle_root: H256,
    pub proof: FinalProof,
}
