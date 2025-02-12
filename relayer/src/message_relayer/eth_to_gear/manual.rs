use std::sync::mpsc::channel;

use primitive_types::H256;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, TxHash};
use gear_rpc_client::GearApi;

use crate::message_relayer::{self, common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor, message_sender::MessageSender,
    },
    EthereumSlotNumber, GSdkArgs, TxHashWithSlot,
}};

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
    let gear_api = GearApi::new(
        &gear_client_args.vara_domain,
        gear_client_args.vara_port,
        gear_client_args.vara_rpc_retries,
    )
    .await
    .expect("Failed to create GearApi");


    let (_handle, sender_requests) = message_relayer::common::gear::checkpoints_extractor::test222(&gear_client_args.vara_domain, gear_client_args.vara_port, gear_client_args.vara_rpc_retries);
    let from_gear_block = {
        let (sender, mut reciever) = tokio::sync::oneshot::channel();
        let request = message_relayer::common::gear::checkpoints_extractor::Request::LatestFinalizedBlock { sender };

        // todo: exit
        sender_requests.send(request).await.unwrap();

        reciever.await.unwrap().unwrap()
    };
    let from_gear_block = {
        let (sender, mut reciever) = tokio::sync::oneshot::channel();
        let request = message_relayer::common::gear::checkpoints_extractor::Request::BlockHashToNumber { hash: from_gear_block, sender };

        // todo: exit
        sender_requests.send(request).await.unwrap();

        reciever.await.unwrap().unwrap()
    };

    let gear_block_listener = GearBlockListener::new(from_gear_block, sender_requests.clone());

    let checkpoints_extractor =
        CheckpointsExtractor::new(checkpoint_light_client_address, sender_requests);

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
