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
    loop {
        match task_inner(&mut this, &mut requests, &responses).await {
            Ok(_) => break,

            Err(e) => {
                log::error!("{e:?}");

                match this.api_provider.reconnect().await {
                    Ok(_) => {
                        log::info!("Reconnected");
                    }

                    Err(err) => {
                        log::error!("Unable to reconnect: {err}");

                        return;
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
) -> anyhow::Result<()> {
    let gear_api = this.api_provider.client();
    while let Some(request) = requests.recv().await {
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

        let proof = gear_api
            .fetch_message_inclusion_merkle_proof(
                request.merkle_root.block_hash,
                message_hash.into(),
            )
            .await?;

        responses.send(Response {
            proof,
            merkle_root: request.merkle_root,
            tx_uuid: request.tx_uuid,
        })?;
    }

    Ok(())
}
