use std::iter;

use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use utils_prometheus::MeteredService;

use crate::message_relayer::{self, common::{
    ethereum::{
        block_listener::BlockListener as EthereumBlockListener,
        message_paid_event_extractor::MessagePaidEventExtractor,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor, message_sender::MessageSender,
    },
    GSdkArgs,
}};

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    message_paid_event_extractor: MessagePaidEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,

    gear_message_sender: MessageSender,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.message_paid_event_extractor.get_sources())
            .chain(self.checkpoints_extractor.get_sources())
            .chain(self.gear_message_sender.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        args: GSdkArgs,
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        bridging_payment_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
    ) -> anyhow::Result<Self> {
        let (_handle, sender_requests) = message_relayer::common::gear::checkpoints_extractor::test222(&args.vara_domain, args.vara_port, args.vara_rpc_retries);
        let from_gear_block = {
            let (sender, mut reciever) = tokio::sync::oneshot::channel();
            let request = message_relayer::common::gear::checkpoints_extractor::Request::LatestFinalizedBlock { sender };

            // todo: exit
            sender_requests.send(request).await?;

            reciever.await??
        };
        let from_gear_block = {
            let (sender, mut reciever) = tokio::sync::oneshot::channel();
            let request = message_relayer::common::gear::checkpoints_extractor::Request::BlockHashToNumber { hash: from_gear_block, sender };

            // todo: exit
            sender_requests.send(request).await?;

            reciever.await??
        };
        let gear_block_listener = GearBlockListener::new(from_gear_block, sender_requests.clone());

        let from_eth_block = eth_api.finalized_block_number().await?;
        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let message_paid_event_extractor = MessagePaidEventExtractor::new(
            eth_api.clone(),
            beacon_client.clone(),
            bridging_payment_address,
        );

        let checkpoints_extractor =
            CheckpointsExtractor::new(checkpoint_light_client_address, sender_requests);

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let gear_message_sender = MessageSender::new(
            args,
            suri,
            eth_api,
            beacon_client,
            historical_proxy_address,
            vft_manager_address,
            route,
            true,
        );

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            message_paid_event_extractor,
            checkpoints_extractor,

            gear_message_sender,
        })
    }

    pub fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run();
        let ethereum_blocks = self.ethereum_block_listener.run();

        let message_paid_events = self.message_paid_event_extractor.run(ethereum_blocks);
        let checkpoints = self.checkpoints_extractor.run(gear_blocks);

        self.gear_message_sender
            .run(message_paid_events, checkpoints);
    }
}
