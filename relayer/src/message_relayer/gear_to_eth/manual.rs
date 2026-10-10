use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::{Accumulator, RootUpdate},
            message_sender::MessageSender,
            status_fetcher::StatusFetcher,
        },
        gear::{
            merkle_proof_fetcher::MerkleProofFetcher,
            message_queued_event_extractor::bind_event_storage,
        },
        message_hash, AuthoritySetId, GearBlockNumber, MessageInBlock, RelayedMerkleRoot,
    },
    gear_to_eth::{
        storage::{GearEventStream, JSONStorage, OutboundLaneIdentity, Storage},
        tx_manager::{Transaction, TransactionManager},
    },
};
use alloy::providers::Provider;
use ethereum_client::EthApi;
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::GearApi;
use primitive_types::U256;
use sails_rs::ActorId;
use std::{cmp, path::Path, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedSender};

const COUNT_BATCH: u64 = 500;

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    mut api_provider: ApiProviderConnection,
    eth_api: EthApi,
    message_nonce: U256,
    gear_block: u32,
    from_eth_block: Option<u64>,
    confirmations: u64,
    governance_admin: ActorId,
    governance_pauser: ActorId,
    storage_path: &Path,
) -> anyhow::Result<()> {
    let destination_genesis = ethereum_client::get_block(eth_api.raw_provider(), 0)
        .await?
        .header
        .hash;
    let storage = Arc::new(JSONStorage::new(storage_path));
    storage
        .bind_outbound_lane(OutboundLaneIdentity {
            destination_chain_id: eth_api.raw_provider().get_chain_id().await?,
            destination_genesis_hash: destination_genesis.0.into(),
            message_queue_address: eth_api.message_queue_address(),
            bridging_payment_address: None,
            fee_exempt_sources: Default::default(),
            sender_address: eth_api.sender_address(),
        })
        .await?;
    bind_event_storage(&mut api_provider, storage.as_ref(), Some(gear_block)).await?;
    anyhow::ensure!(
        storage
            .event_cursor(GearEventStream::Queued)
            .await?
            .is_none()
            && storage.event_cursor(GearEventStream::Paid).await?.is_none(),
        "HOLD: manual relay requires its own journal, not an automatic worker journal"
    );
    let tx_manager = TransactionManager::new(storage.clone());
    tx_manager.load_from_storage().await?;
    eth_api
        .enable_finality_archive(&storage_path.join("ethereum-finality"), destination_genesis)
        .await?;
    tx_manager.verify_completed(&eth_api).await?;
    let gear_api = api_provider.client();
    let finalized = gear_api.latest_finalized_block().await?;
    anyhow::ensure!(
        gear_api.block_hash_to_number(finalized).await? >= gear_block,
        "HOLD: manual source message is not finalized"
    );
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

    {
        let active = tx_manager.transactions.read().await;
        let completed = tx_manager.completed.read().await;
        anyhow::ensure!(
            active.len() + completed.len() <= 1,
            "HOLD: manual journal contains multiple operations"
        );
        anyhow::ensure!(
            tx_manager.failed.read().await.is_empty(),
            "HOLD: manual journal contains failed work"
        );
        for tx in active.values().chain(completed.values()) {
            ensure_manual_target(tx, &message_in_block)?;
        }
        if !completed.is_empty() {
            return Ok(());
        }
    }

    let block_latest = eth_api.verified_finalized_view().await?.block_number();
    let block_range = crate::common::create_range(from_eth_block, block_latest);
    let mut block_from = block_range.from;
    let (merkle_roots_sender, merkle_roots_receiver) = mpsc::unbounded_channel();

    while block_from <= block_range.to {
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

    tx_manager.update_storage().await?;

    let message_sender = MessageSender::new(eth_api.clone());

    let (queued_messages_sender, mut queued_messages_receiver) = mpsc::unbounded_channel();
    let accumulator = Accumulator::new(
        merkle_roots_receiver,
        tx_manager.merkle_roots.clone(),
        storage.clone(),
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

    anyhow::ensure!(
        tx_manager
            .resume(
                &mut accumulator_io,
                &mut proof_fetcher_io,
                &mut message_sender_io,
                &mut status_fetcher_io
            )
            .await?,
        "Manual recovery handlers stopped"
    );
    loop {
        let result = tx_manager
            .process(
                &mut accumulator_io,
                &mut queued_messages_receiver,
                &mut proof_fetcher_io,
                &mut message_sender_io,
                &mut status_fetcher_io,
            )
            .await;
        tx_manager.update_storage().await?;
        let running = result?;
        if !tx_manager.completed.read().await.is_empty() {
            log::info!(
                "Transaction nonce={message_nonce}, block={gear_block} successfully relayed"
            );
            return Ok(());
        }
        anyhow::ensure!(
            tx_manager.failed.read().await.is_empty(),
            "HOLD: manual relay failed; preserve original journal"
        );
        anyhow::ensure!(running, "Manual relay handlers stopped before completion");
    }
}

fn ensure_manual_target(tx: &Transaction, target: &MessageInBlock) -> anyhow::Result<()> {
    anyhow::ensure!(
        tx.message.block == target.block
            && tx.message.block_hash == target.block_hash
            && tx.message.authority_set_id == target.authority_set_id
            && tx.message_hash == message_hash(&target.message),
        "HOLD: manual journal belongs to another source message"
    );
    Ok(())
}

async fn fetch_merkle_roots_in_range(
    eth_api: &EthApi,
    gear_api: &GearApi,
    block_from: u64,
    block_to: u64,
    merkle_roots_sender: &UnboundedSender<RootUpdate>,
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
            .send(RootUpdate::untracked(RelayedMerkleRoot {
                block: GearBlockNumber(merkle_root.block_number as u32),
                block_hash,
                authority_set_id,
                merkle_root: merkle_root.merkle_root,
                timestamp,
            }))
            .expect("Unable to send RelayedMerkleRoot");

        if authority_set_id == message_in_block.authority_set_id
            && merkle_root.block_number >= message_in_block.block.0.into()
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::gear_to_eth::tx_manager::TxStatus;
    use gear_rpc_client::dto::Message;
    use primitive_types::H256;

    #[test]
    fn manual_journal_cannot_switch_message_or_source_block() {
        let target = MessageInBlock {
            message: Message {
                nonce_be: [0; 32],
                source: [1; 32],
                destination: [2; 20],
                payload: vec![3],
            },
            block: GearBlockNumber(10),
            block_hash: H256::repeat_byte(4),
            authority_set_id: AuthoritySetId(5),
        };
        let tx = Transaction::new(target.clone(), TxStatus::WaitForMerkleRoot);
        ensure_manual_target(&tx, &target).unwrap();
        let mut different = target.clone();
        different.message.nonce_be[31] = 1;
        assert!(ensure_manual_target(&tx, &different).is_err());
        different = target.clone();
        different.message.payload.push(4);
        assert!(ensure_manual_target(&tx, &different).is_err());
        different = target.clone();
        different.block_hash = H256::repeat_byte(6);
        assert!(ensure_manual_target(&tx, &different).is_err());
        different = target;
        different.authority_set_id = AuthoritySetId(6);
        assert!(ensure_manual_target(&tx, &different).is_err());
    }
}
