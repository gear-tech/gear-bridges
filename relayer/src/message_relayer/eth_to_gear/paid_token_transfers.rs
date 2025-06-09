use primitive_types::{H160, H256};

use std::{iter, sync::Arc};

use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use utils_prometheus::MeteredService;

use crate::message_relayer::common::{
    ethereum::block_listener::BlockListener as EthereumBlockListener,
    gear::block_listener::BlockListener as GearBlockListener,
};

use super::api_provider::ApiProviderConnection;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    task_manager: Arc<task_manager::TaskManager>,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
        //.chain(self.message_paid_event_extractor.get_sources())
        //.chain(self.checkpoints_extractor.get_sources())
        //.chain(self.gear_message_sender.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        suri: String,
        eth_api: EthApi,
        beacon_client: BeaconClient,
        bridging_payment_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        api_provider: ApiProviderConnection,
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone());

        let from_eth_block = eth_api.finalized_block_number().await?;
        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);
        /*
        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let gear_message_sender = MessageSender::new(
            api_provider,
            suri,
            eth_api,
            beacon_client,
            historical_proxy_address,
            checkpoint_light_client_address,
            vft_manager_address,
            route,
            true,
        );*/

        let task_manager = task_manager::TaskManager::new(
            api_provider.clone(),
            eth_api.clone(),
            beacon_client.clone(),
            bridging_payment_address,
            checkpoint_light_client_address,
            historical_proxy_address,
            vft_manager_address,
            suri,
        );

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            task_manager,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.run().await;

        self.task_manager
            .run(ethereum_blocks, gear_blocks)
            .await
            .unwrap_or_else(|err| {
                log::error!("Relayer task manager failed: {err}");
            });
    }
}

pub mod checkpoint_extractor;
pub mod message_paid_event_extractor;
pub mod proof_composer;
pub mod storage;
pub mod submit_message;
pub mod task_manager;
