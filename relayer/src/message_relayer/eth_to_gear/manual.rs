use primitive_types::H256;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, TxHash};
use tokio::sync::mpsc::unbounded_channel;

use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor, message_sender::MessageSender,
    },
    EthereumSlotNumber, TxHashWithSlot,
};

use super::api_provider::ApiProviderConnection;

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

    let gear_message_sender = MessageSender::new(
        api_provider,
        gear_suri,
        eth_api,
        beacon_client,
        historical_proxy_address,
        checkpoint_light_client_address,
        receiver_address,
        receiver_route,
        true,
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
    gear_message_sender
        .run(deposit_events_receiver, checkpoints)
        .await;
}
