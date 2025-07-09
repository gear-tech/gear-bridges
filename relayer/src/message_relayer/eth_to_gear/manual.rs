use super::{
    api_provider::ApiProviderConnection, message_sender::MessageSender,
    proof_composer::ProofComposer, storage::NoStorage, tx_manager::TransactionManager,
};
use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber, TxHashWithSlot,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use primitive_types::H256;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    mut api_provider: ApiProviderConnection,
    gear_suri: String,

    eth_api: PollingEthApi,
    beacon_client: BeaconClient,

    checkpoint_light_client_address: H256,
    historical_proxy_address: H256,
    receiver_address: H256,

    receiver_route: Vec<u8>,

    tx_hash: TxHash,
    slot: u64,
) {
    let gear_block_listener = GearBlockListener::new(api_provider.clone());

    let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

    let client = api_provider
        .gclient_client(&gear_suri)
        .expect("failed to create gclient");

    let latest_checkpoint =
        super::get_latest_checkpoint(checkpoint_light_client_address, client).await;

    let message_sender = MessageSender::new(
        receiver_address,
        receiver_route,
        historical_proxy_address,
        api_provider.clone(),
        gear_suri.clone(),
    );
    let proof_composer = ProofComposer::new(
        api_provider,
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
            slot_number: EthereumSlotNumber(slot),
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
}
