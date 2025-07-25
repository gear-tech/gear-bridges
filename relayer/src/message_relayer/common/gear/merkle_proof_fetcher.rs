use crate::message_relayer::common::{Data, MessageInBlock, RelayedMerkleRoot};
use gear_common::ApiProviderConnection;
use gear_rpc_client::dto::Message;
use keccak_hash::keccak_256;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub struct MerkleProofFetcher {
    api_provider: ApiProviderConnection,
}

impl MerkleProofFetcher {
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self { api_provider }
    }

    pub fn spawn(
        self,
        messages: UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    ) -> UnboundedReceiver<Data> {
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::task::spawn(task(self, messages, sender));

        receiver
    }
}

async fn task(
    mut this: MerkleProofFetcher,
    mut messages: UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    sender: UnboundedSender<Data>,
) {
    loop {
        match task_inner(&mut this, &mut messages, &sender).await {
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
    messages: &mut UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    sender: &UnboundedSender<Data>,
) -> anyhow::Result<()> {
    let gear_api = this.api_provider.client();
    while let Some((message, merkle_root)) = messages.recv().await {
        let message_hash = message_hash(&message.message);

        log::debug!(
            "Fetch inclusion merkle proof for message with hash {} and nonce {}",
            hex::encode(message_hash),
            hex::encode(message.message.nonce_le)
        );

        let proof = gear_api
            .fetch_message_inclusion_merkle_proof(merkle_root.block_hash, message_hash.into())
            .await?;

        sender.send(Data {
            message,
            relayed_root: merkle_root,
            proof,
        })?;
    }

    Ok(())
}

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}
