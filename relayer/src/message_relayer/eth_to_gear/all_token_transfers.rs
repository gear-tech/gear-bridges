use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;
use std::iter;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use utils_prometheus::MeteredService;

use crate::message_relayer::common::{
    ethereum::{
        block_listener::BlockListener as EthereumBlockListener,
        deposit_event_extractor::DepositEventExtractor,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor, message_sender::MessageSender,
    },
};

use super::api_provider::ApiProviderConnection;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    deposit_event_extractor: DepositEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,

    gear_message_sender: MessageSender,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.deposit_event_extractor.get_sources())
            .chain(self.checkpoints_extractor.get_sources())
            .chain(self.gear_message_sender.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        erc20_manager_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        api_provider: ApiProviderConnection,
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone());

        let from_eth_block = eth_api.finalized_block_number().await?;
        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let deposit_event_extractor = DepositEventExtractor::new(
            eth_api.clone(),
            beacon_client.clone(),
            erc20_manager_address,
        );

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let gear_message_sender = MessageSender::new(
            api_provider.clone(),
            suri,
            eth_api,
            beacon_client,
            historical_proxy_address,
            checkpoint_light_client_address,
            vft_manager_address,
            route,
            true,
        );

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            deposit_event_extractor,
            checkpoints_extractor,

            gear_message_sender,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.run().await;

        let deposit_events = self.deposit_event_extractor.run(ethereum_blocks).await;
        let checkpoints = self.checkpoints_extractor.run(gear_blocks).await;

        self.gear_message_sender
            .run(deposit_events, checkpoints)
            .await;
    }
}
