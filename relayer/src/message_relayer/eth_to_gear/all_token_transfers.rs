use super::{
    message_sender::MessageSender,
    proof_composer::ProofComposer,
    storage::{JSONStorage, Storage},
    tx_manager::TransactionManager,
};
use crate::message_relayer::common::{
    ethereum::{
        block_listener::BlockListener as EthereumBlockListener,
        deposit_event_extractor::DepositEventExtractor,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener, block_storage::NoStorage,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gear_common::ApiProviderConnection;
use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;
use std::{iter, sync::Arc};
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    deposit_event_extractor: DepositEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,
    latest_checkpoint: Option<EthereumSlotNumber>,

    proof_composer: ProofComposer,
    gear_message_sender: MessageSender,

    storage: Arc<dyn Storage>,

    tx_manager: TransactionManager,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.deposit_event_extractor.get_sources())
            .chain(self.checkpoints_extractor.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        suri: String,
        eth_api: PollingEthApi,
        beacon_client: BeaconClient,
        erc20_manager_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        mut api_provider: ApiProviderConnection,
        storage_path: String,
        genesis_time: u64,
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone(), Arc::new(NoStorage));

        let from_eth_block = eth_api.finalized_block().await?.header.number;
        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let storage = Arc::new(JSONStorage::new(storage_path));

        let deposit_event_extractor = DepositEventExtractor::new(
            eth_api.clone(),
            erc20_manager_address,
            storage.clone(),
            genesis_time,
        );

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

        let client = api_provider
            .gclient_client(&suri)
            .expect("failed to construct gclient");

        let latest_checkpoint =
            super::get_latest_checkpoint(checkpoint_light_client_address, client).await;

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let gear_message_sender = MessageSender::new(
            vft_manager_address,
            route,
            historical_proxy_address,
            api_provider.clone(),
            suri.clone(),
        );

        let proof_composer = ProofComposer::new(
            api_provider,
            beacon_client,
            eth_api,
            historical_proxy_address,
            suri,
        );

        let tx_manager = TransactionManager::new(storage.clone());

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            deposit_event_extractor,
            checkpoints_extractor,
            latest_checkpoint,

            proof_composer,
            gear_message_sender,

            storage,
            tx_manager,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.spawn();

        let deposit_events = self.deposit_event_extractor.run(ethereum_blocks).await;

        let checkpoints = self
            .checkpoints_extractor
            .run(gear_blocks, self.latest_checkpoint)
            .await;
        let proof_composer = self.proof_composer.run(checkpoints);
        let message_sender = self.gear_message_sender.run();

        if let Err(err) = self.storage.load(&self.tx_manager).await {
            log::error!("Failed to load transactions and blocks from storage: {err:?}");
        }

        if let Err(err) = self
            .tx_manager
            .run(deposit_events, proof_composer, message_sender)
            .await
        {
            log::error!("Transaction manager exited with error: {err:?}");
        }
    }
}
