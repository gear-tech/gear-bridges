use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator, message_sender::MessageSender, status_fetcher::StatusFetcher,
        },
        gear::merkle_proof_fetcher::MerkleProofFetcher,
        AuthoritySetId, GearBlockNumber, MessageInBlock, RelayedMerkleRoot,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
    gear_to_eth::{
        storage::NoStorage,
        tx_manager::{TransactionManager, TxStatus},
    },
};
use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use primitive_types::U256;
use sails_rs::ActorId;
use std::{cmp, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedSender};

const COUNT_BATCH: u64 = 500;

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    api_provider: ApiProviderConnection,
    eth_api: EthApi,
    message_nonce: U256,
    gear_block: u32,
    from_eth_block: Option<u64>,
    confirmations: u64,
    governance_admin: ActorId,
    governance_pauser: ActorId,
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
        .find(|m| U256::from_big_endian(&m.nonce_be) == message_nonce)
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
    let mut block_from = block_range.from;
    let (merkle_roots_sender, merkle_roots_receiver) = mpsc::unbounded_channel();

    while block_from < block_range.to {
        let block_to = block_from + COUNT_BATCH;
        let block_to = cmp::min(block_to, block_range.to);

        if fetch_merkle_roots_in_range(
            &eth_api,
            &gear_api,
            block_from,
            block_to,
            &merkle_roots_sender,
            &message_in_block,
        )
        .await
        {
            break;
        }

        block_from = block_to + 1;
    }

    let storage = Arc::new(NoStorage::new());
    let tx_manager = TransactionManager::new(storage.clone());

    let message_sender = MessageSender::new(1, eth_api.clone());

    let (queued_messages_sender, mut queued_messages_receiver) = mpsc::unbounded_channel();
    let accumulator = Accumulator::new(
        merkle_roots_receiver,
        tx_manager.merkle_roots.clone(),
        governance_admin,
        governance_pauser,
        eth_api.clone(),
    );
    let mut accumulator_io = accumulator.spawn();
    let mut proof_fetcher_io = MerkleProofFetcher::new(api_provider).spawn();

    let mut message_sender_io = message_sender.spawn();

    queued_messages_sender
        .send(message_in_block)
        .expect("Failed to send message to channel");

    let status_fetcher = StatusFetcher::new(eth_api, confirmations);
    let mut status_fetcher_io = status_fetcher.spawn();

    loop {
        match tx_manager
            .process(
                &mut accumulator_io,
                &mut queued_messages_receiver,
                &mut proof_fetcher_io,
                &mut message_sender_io,
                &mut status_fetcher_io,
            )
            .await
        {
            Ok(true) => {
                if !tx_manager.completed.read().await.is_empty() {
                    log::info!("Transaction nonce={message_nonce}, block={gear_block} successfully relayed");

                    return;
                } else if !tx_manager.failed.read().await.is_empty() {
                    log::error!(
                        "Failed to relay transaction nonce={message_nonce}, block={gear_block}: {}",
                        tx_manager.failed.read().await.first_key_value().unwrap().1
                    );

                    return;
                }

                let binding = tx_manager.transactions.read().await;
                let Some((_key, tx)) = binding.first_key_value() else {
                    continue;
                };

                if let TxStatus::Completed = tx.status {
                    return;
                }
            }

            Ok(false) => {
                log::warn!("Trasaction manager exiting");
                return;
            }

            Err(err) => {
                log::error!("Error occurred while processing transaction manager: {err}");
                return;
            }
        }
    }
}

async fn fetch_merkle_roots_in_range(
    eth_api: &EthApi,
    gear_api: &GearApi,
    block_from: u64,
    block_to: u64,
    merkle_roots_sender: &UnboundedSender<RelayedMerkleRoot>,
    message_in_block: &MessageInBlock,
) -> bool {
    log::info!("Fetch merkle roots in the Ethereum blocks range [{block_from}; {block_to}]",);

    let merkle_roots = eth_api
        .fetch_merkle_roots_in_range(block_from, block_to)
        .await
        .expect("Unable to fetch merkle roots");

    for (merkle_root, block_number_eth) in merkle_roots
        .into_iter()
        .filter_map(|(merkle_root, block)| block.map(|block| (merkle_root, block)))
    {
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

        let timestamp = eth_api
            .get_block_timestamp(block_number_eth)
            .await
            .expect("failed to get Ethereum block timestamp");

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
                timestamp,
            })
            .expect("Unable to send RelayedMerkleRoot");

        if authority_set_id == message_in_block.authority_set_id
            && merkle_root.block_number >= message_in_block.block.0.into()
        {
            return true;
        }
    }

    false
}
