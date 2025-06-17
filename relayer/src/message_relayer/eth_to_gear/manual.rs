use primitive_types::H256;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, TxHash};
use tokio::sync::mpsc::unbounded_channel;

use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber, TxHashWithSlot,
};

use super::{
    api_provider::ApiProviderConnection, message_sender::MessageSender,
    proof_composer::ProofComposer, tx_manager::TransactionManager,
};

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    api_provider: ApiProviderConnection,
    gear_suri: String,

    eth_api: EthApi,
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

    let checkpoints = checkpoints_extractor.run(gear_blocks).await;

    let tx_manager = TransactionManager::new(None);

    let message_sender = message_sender.run();
    let proof_composer = proof_composer.run(checkpoints);

    match tx_manager
        .run(deposit_events_receiver, proof_composer, message_sender)
        .await
    {
        Ok(_) => {}

        Err(err) => {
            log::error!("Transasction manager failed with error: {err:?}");
        }
    }

    drop(deposit_events_sender);
}
