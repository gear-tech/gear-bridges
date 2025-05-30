use primitive_types::U256;

use ethereum_client::EthApi;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::message_relayer::{
    common::{
        ethereum::{accumulator::Accumulator, message_sender::MessageSender},
        AuthoritySetId, GearBlockNumber, MessageInBlock, RelayedMerkleRoot,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
};

pub async fn relay(
    api_provider: ApiProviderConnection,
    eth_api: EthApi,
    message_nonce: U256,
    gear_block: u32,
    from_eth_block: Option<u64>,
) -> UnboundedSender<MessageInBlock> {
    let block_latest = eth_api
        .block_number()
        .await
        .expect("Failed to get the latest block number on Ethereum");
    let from_eth_block = from_eth_block.unwrap_or(block_latest);

    let merkle_roots = eth_api
        .fetch_merkle_roots_in_range(from_eth_block, block_latest)
        .await
        .expect("Unable to fetch merkle roots");

    let (queued_messages_sender, queued_messages_receiver) = mpsc::unbounded_channel();
    if merkle_roots.is_empty() {
        log::info!("Found no merkle roots");

        return queued_messages_sender;
    }

    let gear_api = api_provider.client();
    let (sender, receiver) = mpsc::unbounded_channel();
    for (merkle_root, _block_number_eth) in merkle_roots {
        let block_hash = gear_api
            .block_number_to_hash(merkle_root.block_number as u32)
            .await
            .expect("Unable to get hash for the block number");

        let authority_set_id = AuthoritySetId(
            gear_api
                .signed_by_authority_set_id(block_hash)
                .await
                .expect("Unable to get AuthoritySetId"),
        );

        log::info!(
            "Found merkle root for gear block #{} and era #{}",
            merkle_root.block_number,
            authority_set_id
        );

        sender
            .send(RelayedMerkleRoot {
                block: GearBlockNumber(merkle_root.block_number as u32),
                block_hash,
                authority_set_id,
                merkle_root: merkle_root.merkle_root,
            })
            .expect("Unable to send RelayedMerkleRoot");
    }

    let gear_block_hash = gear_api
        .block_number_to_hash(gear_block)
        .await
        .expect("Failed to fetch block hash by number");

    let message_queued_events = gear_api
        .message_queued_events(gear_block_hash)
        .await
        .expect("Failed to fetch MessageQueued events from gear block");

    let message = message_queued_events
        .into_iter()
        .find(|m| U256::from_little_endian(&m.nonce_le) == message_nonce)
        .unwrap_or_else(|| {
            panic!("Message with nonce {message_nonce} is not found in gear block {gear_block}")
        });

    let message_in_block = MessageInBlock {
        message,
        block: GearBlockNumber(gear_block),
        block_hash: gear_block_hash,
        authority_set_id: AuthoritySetId(
            gear_api
                .signed_by_authority_set_id(gear_block_hash)
                .await
                .expect("Unable to get authority set id"),
        ),
    };

    let message_sender = MessageSender::new(eth_api, api_provider);

    let accumulator = Accumulator::new();
    let channel_messages = accumulator.run(queued_messages_receiver, receiver).await;
    message_sender.run(channel_messages).await;

    queued_messages_sender
        .send(message_in_block)
        .expect("Failed to send message to channel");

    queued_messages_sender
}
