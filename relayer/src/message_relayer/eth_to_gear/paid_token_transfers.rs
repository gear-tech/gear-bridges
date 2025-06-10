use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;
use std::{iter, sync::Arc};
use utils_prometheus::MeteredService;

use crate::message_relayer::{
    common::{
        ethereum::{
            block_listener::BlockListener as EthereumBlockListener,
            message_paid_event_extractor::MessagePaidEventExtractor,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            checkpoints_extractor::CheckpointsExtractor,
        },
    },
    eth_to_gear::paid_token_transfers::storage::Storage,
};

use super::api_provider::ApiProviderConnection;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    message_sender: message_sender::MessageSender,
    message_paid_event_extractor: MessagePaidEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,
    proof_composer: proof_composer::ProofComposerTask,

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
        storage: Storage,
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone());

        let from_eth_block = eth_api.finalized_block_number().await?;
        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);
        let message_paid_event_extractor = MessagePaidEventExtractor::new(
            eth_api.clone(),
            beacon_client.clone(),
            bridging_payment_address,
        );

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();
        let message_sender = message_sender::MessageSender::new(
            vft_manager_address,
            route,
            historical_proxy_address,
            api_provider.clone(),
            suri.clone(),
        );

        let proof_composer = proof_composer::ProofComposerTask::new(
            api_provider.clone(),
            beacon_client.clone(),
            eth_api.clone(),
            historical_proxy_address,
            suri.clone(),
        );

        let task_manager = task_manager::TaskManager::new(storage);

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            task_manager,
            checkpoints_extractor,
            message_paid_event_extractor,
            message_sender,
            proof_composer,
        })
    }

    pub async fn run(self, resume_from_storage: bool) {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.run().await;

        let message_paid_events = self.message_paid_event_extractor.run(ethereum_blocks).await;

        let checkpoints = self.checkpoints_extractor.run(gear_blocks).await;
        let proof_composer_io = self.proof_composer.run(checkpoints);
        let msg_sender_io = self.message_sender.run();

        self.task_manager
            .run(
                resume_from_storage,
                proof_composer_io,
                message_paid_events,
                msg_sender_io,
            )
            .await
            .unwrap_or_else(|err| {
                log::error!("Relayer task manager failed: {err}");
            });
    }
}

pub mod proof_composer;
pub mod storage;
//pub mod submit_message;
pub mod message_sender;
pub mod task_manager;
