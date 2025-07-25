use super::{
    message_sender::MessageSender,
    proof_composer::ProofComposer, storage::NoStorage, tx_manager::TransactionManager,
};
use gear_common::ApiProviderConnection;
use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber, TxHashWithSlot,
};
use alloy::{network::TransactionResponse, providers::Provider};
use anyhow::{Context, Result as AnyResult};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use ethereum_common::SECONDS_PER_SLOT;
use primitive_types::H256;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    mut provider_connection: ApiProviderConnection,
    gear_suri: String,

    eth_api: PollingEthApi,
    beacon_client: BeaconClient,

    checkpoint_light_client_address: H256,
    historical_proxy_address: H256,
    receiver_address: H256,

    receiver_route: Vec<u8>,

    tx_hash: TxHash,
) -> AnyResult<()> {
    let tx = eth_api
        .get_transaction_by_hash(tx_hash)
        .await?
        .context(r#"Transaction "{tx_hash}" is None"#)?;
    let block_number = tx.block_number().context("Block number is None")?;
    let block_timestamp = eth_api.get_block(block_number).await?.header.timestamp;

    let genesis_time = beacon_client
        .get_genesis()
        .await
        .context("Failed to fetch chain genesis")?
        .data
        .genesis_time;

    log::info!("Genesis time: {genesis_time}");
    let slot_number =
        EthereumSlotNumber(block_timestamp.saturating_sub(genesis_time) / SECONDS_PER_SLOT);
    log::info!(r#"Slot number of the transaction ("{tx_hash}") block is {slot_number}"#);

    let gear_block_listener = GearBlockListener::new(
        provider_connection.clone(),
        Arc::new(crate::message_relayer::common::gear::block_storage::NoStorage),
    );

    let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

    let client = provider_connection
        .gclient_client(&gear_suri)
        .context("Failed to create gclient")?;

    let latest_checkpoint =
        super::get_latest_checkpoint(checkpoint_light_client_address, client).await;

    log::debug!("latest_checkpoint = {latest_checkpoint:?}");

    let message_sender = MessageSender::new(
        receiver_address,
        receiver_route,
        historical_proxy_address,
        provider_connection.clone(),
        gear_suri.clone(),
    );
    let proof_composer = ProofComposer::new(
        provider_connection,
        beacon_client,
        eth_api,
        historical_proxy_address,
        gear_suri.clone(),
    );
    let [gear_blocks] = gear_block_listener.run().await;
    let (deposit_events_sender, deposit_events_receiver) = unbounded_channel();

    deposit_events_sender
        .send(TxHashWithSlot {
            tx_hash,
            slot_number,
        })
        .expect("Failed to send message to channel");

    let checkpoints = checkpoints_extractor
        .run(gear_blocks, latest_checkpoint)
        .await;

    let tx_manager = TransactionManager::new(Arc::new(NoStorage::new()));

    let message_sender = message_sender.run();
    let proof_composer = proof_composer.run(checkpoints);

    if let Err(err) = tx_manager
        .run(deposit_events_receiver, proof_composer, message_sender)
        .await
    {
        log::error!("Transasction manager failed with error: {err:?}");
    }

    drop(deposit_events_sender);

    Ok(())
}
