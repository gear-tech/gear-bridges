use std::sync::mpsc::channel;

use primitive_types::H256;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, TxHash};
use gear_rpc_client::GearApi;

use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor, message_sender::MessageSender,
    },
    EthereumSlotNumber, GSdkArgs, TxHashWithSlot,
};

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    gear_client_args: GSdkArgs,
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
    let from_gear_block = gear_api
        .latest_finalized_block()
        .await
        .expect("Failed to fetch latest finalized block");

    let from_gear_block = gear_api
        .block_hash_to_number(from_gear_block)
        .await
        .expect("Failed to fetch block number by hash");

    let gear_block_listener = GearBlockListener::new(gear_client_args.clone(), from_gear_block);

    let checkpoints_extractor =
        CheckpointsExtractor::new(gear_client_args.clone(), checkpoint_light_client_address);

    let gear_message_sender = MessageSender::new(
        gear_client_args,
        gear_suri,
        eth_api,
        beacon_client,
        historical_proxy_address,
        receiver_address,
        receiver_route,
        false,
    );

    let [gear_blocks] = gear_block_listener.run();
    let (deposit_events_sender, deposit_events_receiver) = channel();
    let checkpoints = checkpoints_extractor.run(gear_blocks);
    gear_message_sender.run(deposit_events_receiver, checkpoints);

    deposit_events_sender
        .send(TxHashWithSlot {
            tx_hash,
            slot_number: EthereumSlotNumber(slot),
        })
        .expect("Failed to send message to channel");
}
