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

        // ~13 minutes worth of blocks
        // Why 13 minutes aka 256 blocks? Under heavy load (many merkle-root changes == many finality proof requests)
        // we don't want to generate proof for every single request since it would suffice to generate
        // the proof for the latest block in each authority set and use it for all blocks
        // in that set.
        const BATCH_SIZE: usize = 256;

        let mut batch_vec = Vec::with_capacity(BATCH_SIZE);
        // Group requests by authority set ID, storing all blocks for each set
        let mut authority_groups: std::collections::HashMap<u64, Vec<Request>> =
            std::collections::HashMap::new();

        loop {
            let n = requests.recv_many(&mut batch_vec, BATCH_SIZE).await;
            if n == 0 {
                log::info!("Requests channel closed, exiting");
                break;
            }

            if n == 1 {
                log::info!("Only one request received, processing it immediately");
                let request = batch_vec.pop().unwrap();
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
                    log::warn!("Response channel closed, exiting");
                    return Ok(());
                }

                continue;
            }

            log::info!("Received {n} requests, grouping by authority set...");

            for request in batch_vec.drain(..) {
                let authority_set_id = gear_api.authority_set_id(request.block_hash).await?;
                authority_groups
                    .entry(authority_set_id)
                    .or_default()
                    .push(request);
            }

            for (authority_set_id, mut requests) in authority_groups.drain() {
                // Sort requests by block number to find the latest one
                requests.sort_by_key(|req| req.block_number);
                let latest_request = requests.pop().expect("At least one request per group");
                log::info!(
                    "Proving finality for latest block #{} with authority set #{} and merkle-root {} (will apply to {} blocks)",
                    latest_request.block_number,
                    authority_set_id,
                    latest_request.merkle_root,
                    requests.len()
                );

                // Generate proof for the latest block
                let proof = self
                    .generate_proof(
                        &gear_api,
                        latest_request.block_number,
                        latest_request.block_hash,
                        latest_request.merkle_root,
                        latest_request.inner_proof,
                    )
                    .await?;

                // send response to the latest request first
                if responses
                    .send(Response {
                        block_number: latest_request.block_number,
                        merkle_root: latest_request.merkle_root,
                        proof: proof.clone(),
                    })
                    .is_err()
                {
                    log::warn!("Response channel closed, exiting");
                    return Ok(());
                }

                // Send response for all blocks in this authority set using the same proof
                for req in requests {
                    if responses
                        .send(Response {
                            block_number: req.block_number,
                            merkle_root: req.merkle_root,
                            proof: proof.clone(),
                        })
                        .is_err()
                    {
                        log::warn!("Response channel closed, exiting");
                        return Ok(());
                    }
                }
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

#[derive(Clone)]
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
