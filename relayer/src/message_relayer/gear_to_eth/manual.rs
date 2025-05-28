use alloy::providers::{Provider, PendingTransactionBuilder};
use primitive_types::U256;
use ethereum_client::EthApi;
use tokio::sync::mpsc;

use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator, block_listener::BlockListener as EthereumBlockListener,
            merkle_root_extractor::MerkleRootExtractor, message_sender::MessageSender,
            merkle_proof_fetcher::MerkleProofFetcher,
        },
        AuthoritySetId, GearBlockNumber, MessageInBlock,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
};

const COUNT_CONFIRMATIONS: u64 = 5;

pub async fn relay(
    api_provider: ApiProviderConnection,
    eth_api: EthApi,
    message_nonce: U256,
    gear_block: u32,
    from_eth_block: Option<u64>,
) {
    let from_eth_block = if let Some(block) = from_eth_block {
        block
    } else {
        eth_api
            .finalized_block_number()
            .await
            .expect("Failed to get finalized block number on ethereum")
    };

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
            panic!(
                "Message with nonce {} is not found in gear block {}",
                message_nonce, gear_block
            )
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

    let (queued_messages_sender, queued_messages_receiver) = mpsc::unbounded_channel();

    let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);
    let merkle_root_extractor = MerkleRootExtractor::new(eth_api.clone(), api_provider.clone());
    let message_sender = MessageSender::new(1, eth_api.clone());

    let ethereum_blocks = ethereum_block_listener.run().await;
    let merkle_roots = merkle_root_extractor.run(ethereum_blocks).await;
    let accumulator = Accumulator::new();
    let channel_messages = accumulator
        .run(queued_messages_receiver, merkle_roots)
        .await;
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
        .with_required_confirmations(COUNT_CONFIRMATIONS)
        .watch()
        .await;
    
    log::info!("Result for message {message_nonce:#x} after {COUNT_CONFIRMATIONS} confirmation(s): {result:?}");
}
