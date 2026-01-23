use crate::message_relayer::{
    common::RelayedMerkleRoot, eth_to_gear::api_provider::ApiProviderConnection,
};
use gear_rpc_client::dto::MerkleProof;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

pub struct Request {
    pub tx_uuid: Uuid,
    pub message_block: u32,
    pub message_hash: [u8; 32],
    pub message_nonce: [u8; 32],
    pub merkle_root: RelayedMerkleRoot,
}

pub struct Response {
    pub proof: MerkleProof,
    pub merkle_root: RelayedMerkleRoot,
    pub tx_uuid: Uuid,
}

pub struct MerkleRootFetcherIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl MerkleRootFetcherIo {
    pub fn send_request(
        &self,
        tx_uuid: Uuid,
        message_block: u32,
        message_hash: [u8; 32],
        message_nonce: [u8; 32],
        merkle_root: RelayedMerkleRoot,
    ) -> bool {
        let request = Request {
            tx_uuid,
            message_block,
            message_hash,
            message_nonce,
            merkle_root,
        };
        self.requests.send(request).is_ok()
    }

    pub async fn recv_message(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

pub struct MerkleProofFetcher {
    api_provider: ApiProviderConnection,
}

impl MerkleProofFetcher {
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self { api_provider }
    }

    pub fn spawn(self) -> MerkleRootFetcherIo {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (resp_tx, resp_rx) = mpsc::unbounded_channel();
        tokio::task::spawn(task(self, req_rx, resp_tx));

        MerkleRootFetcherIo {
            requests: req_tx,
            responses: resp_rx,
        }
    }
}

async fn task(
    mut this: MerkleProofFetcher,
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) {
    let mut pending_request: Option<Request> = None;

    loop {
        match task_inner(&mut this, &mut requests, &responses, &mut pending_request).await {
            Ok(_) => break,

            Err(e) => {
                log::error!("{e:?}");
                
                loop {
                    match this.api_provider.reconnect().await {
                        Ok(_) => {
                            log::info!("Reconnected");
                            break;
                        }
                        Err(err) => {
                            log::error!("Unable to reconnect: {err}. Retrying in 5s...");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        }
    }
}

async fn task_inner(
    this: &mut MerkleProofFetcher,
    requests: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
    pending_request: &mut Option<Request>,
) -> anyhow::Result<()> {
    // Re-create client if needed inside loop or just use `this.api_provider` which will have new client after reconnect.
    // However, `this.api_provider.client()` returns a cloned `GearApi`. 
    // `task_inner` should probably get the client fresh each time if it changes, 
    // but `ApiProviderConnection` handles internal client update on reconnect? 
    // Looking at block_listener, it calls `self.api_provider.client()` in loop. 
    // So here we should probably not cache `gear_api` outside the loop if it becomes stale?
    // Actually `ApiProviderConnection` likely wraps `Arc<RwLock>` or similar if it's dynamic, 
    // or we just get a new one. Let's assume `this.api_provider.client()` is cheap or we just call it.
    
    loop {
        let request = if let Some(req) = pending_request.take() {
            req
        } else {
             match requests.recv().await {
                Some(req) => req,
                None => return Ok(()),
             }
        };

        let message_hash = request.message_hash;
        log::info!(
            "Fetch inclusion merkle proof for message at block #{}, message hash={}, message nonce={}, merkle-root {} at block #{}({})",
            request.message_block,
            hex::encode(message_hash),
            hex::encode(request.message_nonce),
            request.merkle_root.merkle_root,
            request.merkle_root.block,
            request.merkle_root.block_hash,
        );

        let gear_api = this.api_provider.client();
        let proof_res = gear_api
            .fetch_message_inclusion_merkle_proof(
                request.merkle_root.block_hash,
                message_hash.into(),
            )
            .await;

         match proof_res {
            Ok(proof) => {
                 responses.send(Response {
                    proof,
                    merkle_root: request.merkle_root,
                    tx_uuid: request.tx_uuid,
                })?;
            }
            Err(e) => {
                *pending_request = Some(request);
                return Err(e.into());
            }
        }
    }
}
