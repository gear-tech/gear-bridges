use alloy::providers::{PendingTransactionBuilder, Provider};
use ethereum_client::EthApi;
use primitive_types::U256;
use tokio::sync::mpsc;

use crate::message_relayer::{
    common::{
        ethereum::{accumulator::Accumulator, message_sender::MessageSender},
        gear::merkle_proof_fetcher::MerkleProofFetcher,
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
    confirmations: u64,
) {
    let gear_api = api_provider.client();
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

    let authority_set_id = AuthoritySetId(
        gear_api
            .signed_by_authority_set_id(gear_block_hash)
            .await
            .expect("Unable to get authority set id"),
    );
    log::debug!("AuthoritySetId for the message is {authority_set_id}");
    let message_in_block = MessageInBlock {
        message,
        block: GearBlockNumber(gear_block),
        block_hash: gear_block_hash,
        authority_set_id,
    };

    let block_latest = eth_api
        .block_number()
        .await
        .expect("Failed to get the latest block number on Ethereum");
    let block_range = crate::common::create_range(from_eth_block, block_latest);
    let merkle_roots = eth_api
        .fetch_merkle_roots_in_range(block_range.from, block_range.to)
        .await
        .expect("Unable to fetch merkle roots");

    if merkle_roots.is_empty() {
        log::info!("Found no merkle roots");

        return;
    }

    let (merkle_roots_sender, merkle_roots_receiver) = mpsc::unbounded_channel();
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

        merkle_roots_sender
            .send(RelayedMerkleRoot {
                block: GearBlockNumber(merkle_root.block_number as u32),
                block_hash,
                authority_set_id,
                merkle_root: merkle_root.merkle_root,
            })
            .expect("Unable to send RelayedMerkleRoot");

        if authority_set_id == message_in_block.authority_set_id
            && merkle_root.block_number >= gear_block.into()
        {
            break;
        }
    }

    let message_sender = MessageSender::new(1, eth_api.clone());

    let (queued_messages_sender, queued_messages_receiver) = mpsc::unbounded_channel();
    let accumulator = Accumulator::new(merkle_roots_receiver, queued_messages_receiver);
    let channel_messages = accumulator.spawn();
    let channel_message_data = MerkleProofFetcher::new(api_provider).spawn(channel_messages);

    let (sender, mut receiver) = mpsc::unbounded_channel();
    message_sender.spawn(channel_message_data, sender);

    queued_messages_sender
        .send(message_in_block)
        .expect("Failed to send message to channel");

    let Some((tx_hash, _message)) = receiver.recv().await else {
        log::info!("Unable to receive transaction data for a message {message_nonce:#x}");
        return;
    };

    let provider = eth_api.raw_provider().root().clone();
    let result = PendingTransactionBuilder::new(provider, tx_hash)
        .with_required_confirmations(confirmations)
        .watch()
        .await;

    log::info!(
        "Result for message {message_nonce:#x} after {confirmations} confirmation(s): {result:?}"
    );
}
