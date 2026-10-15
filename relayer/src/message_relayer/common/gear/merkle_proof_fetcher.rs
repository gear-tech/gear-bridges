use crate::{message_relayer::common::RelayedMerkleRoot, rpc};
use gear_common::api_provider::ApiProviderConnection;
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
    let mut current = None;
    loop {
        match task_inner(&mut this, &mut requests, &responses, &mut current).await {
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
    current: &mut Option<Request>,
) -> anyhow::Result<()> {
    loop {
        let api_provider = &mut this.api_provider;
        if !deliver_next(
            current,
            requests,
            responses,
            move |block_hash, message_hash| {
                rpc::retry_gear(
                    api_provider,
                    "message inclusion merkle proof",
                    move |gear_api| async move {
                        gear_api
                            .fetch_message_inclusion_merkle_proof(block_hash, message_hash.into())
                            .await
                    },
                )
            },
        )
        .await?
        {
            return Ok(());
        }
    }
}

async fn deliver_next<F, Fut>(
    current: &mut Option<Request>,
    requests: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
    fetch: F,
) -> anyhow::Result<bool>
where
    F: FnOnce(primitive_types::H256, [u8; 32]) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<MerkleProof>>,
{
    if current.is_none() {
        *current = requests.recv().await;
    }
    let Some(request) = current.as_ref() else {
        return Ok(false);
    };
    log::info!(
        "Fetch inclusion merkle proof for transaction {}, message at block #{}, nonce={}, merkle-root {} at #{}({})",
        request.tx_uuid, request.message_block, hex::encode(request.message_nonce),
        request.merkle_root.merkle_root, request.merkle_root.block, request.merkle_root.block_hash,
    );
    let proof = fetch(request.merkle_root.block_hash, request.message_hash).await?;
    responses.send(Response {
        proof,
        merkle_root: request.merkle_root,
        tx_uuid: request.tx_uuid,
    })?;
    *current = None;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::common::{AuthoritySetId, GearBlockNumber};
    use primitive_types::H256;

    fn request(tx_uuid: Uuid, hash: [u8; 32]) -> Request {
        Request {
            tx_uuid,
            message_block: 42,
            message_hash: hash,
            message_nonce: [7; 32],
            merkle_root: RelayedMerkleRoot {
                block: GearBlockNumber(43),
                block_hash: H256::repeat_byte(8),
                timestamp: 123,
                authority_set_id: AuthoritySetId(1),
                merkle_root: H256(hash),
            },
        }
    }

    async fn single_leaf_proof(_: H256, message_hash: [u8; 32]) -> anyhow::Result<MerkleProof> {
        Ok(MerkleProof {
            root: message_hash,
            proof: vec![],
            num_leaves: 1,
            leaf_index: 0,
        })
    }

    #[tokio::test]
    async fn reconnect_retains_original_uuid_until_one_response_is_delivered() {
        let (sender, mut requests) = mpsc::unbounded_channel();
        let (responses, mut received) = mpsc::unbounded_channel();
        let original = Uuid::new_v4();
        let later = Uuid::new_v4();
        sender.send(request(original, [3; 32])).unwrap();
        sender.send(request(later, [4; 32])).unwrap();
        drop(sender);
        let mut current = None;
        for _ in 0..2 {
            let failed = deliver_next(&mut current, &mut requests, &responses, |_, _| async {
                anyhow::bail!("RPC reconnect failed before proof delivery")
            })
            .await;
            assert!(failed.is_err());
            assert_eq!(current.as_ref().unwrap().tx_uuid, original);
            assert!(received.try_recv().is_err());
        }
        assert!(
            deliver_next(&mut current, &mut requests, &responses, single_leaf_proof)
                .await
                .unwrap()
        );
        let first = received.try_recv().unwrap();
        assert_eq!(first.tx_uuid, original);
        assert_eq!(first.proof.root, [3; 32]);
        assert!(
            deliver_next(&mut current, &mut requests, &responses, single_leaf_proof)
                .await
                .unwrap()
        );
        let second = received.try_recv().unwrap();
        assert_eq!(second.tx_uuid, later);
        assert_eq!(second.proof.root, [4; 32]);
        assert!(
            !deliver_next(&mut current, &mut requests, &responses, single_leaf_proof)
                .await
                .unwrap()
        );
        assert!(received.try_recv().is_err());
    }

    #[tokio::test]
    async fn closed_response_channel_retains_request_and_exposes_failure() {
        let (sender, mut requests) = mpsc::unbounded_channel();
        let (responses, received) = mpsc::unbounded_channel();
        let original = Uuid::new_v4();
        sender.send(request(original, [3; 32])).unwrap();
        drop(received);
        let mut current = None;
        assert!(
            deliver_next(&mut current, &mut requests, &responses, single_leaf_proof)
                .await
                .is_err()
        );
        assert_eq!(current.as_ref().unwrap().tx_uuid, original);
    }
}
