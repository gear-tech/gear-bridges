use super::{
    message_sender, proof_composer,
    storage::{JSONStorage, Storage},
    tx_manager,
};
use crate::message_relayer::common::{
    ethereum::{
        self, block_listener::BlockListener as EthereumBlockListener,
        block_storage::UnprocessedBlockStorage,
        message_paid_event_extractor::MessagePaidEventExtractor,
        transaction_data_extractor::TransactionDataExtractor,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor,
    },
    web_request::EthTransaction,
    EthereumSlotNumber, TxHashWithSlot,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gear_common::api_provider::ApiProviderConnection;
use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;
use std::{iter, sync::Arc};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tx_manager::TransactionManager;
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    message_paid_event_extractor: MessagePaidEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,
    latest_checkpoint: Option<EthereumSlotNumber>,

    message_sender: message_sender::MessageSender,
    proof_composer: proof_composer::ProofComposer,
    tx_manager: TransactionManager,

    transaction_data_extractor: Option<TransactionDataExtractor>,
    tx_events_sender: Option<UnboundedSender<TxHashWithSlot>>,
    tx_events_receiver: Option<UnboundedReceiver<TxHashWithSlot>>,

    storage: Arc<dyn Storage>,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.message_paid_event_extractor.get_sources())
            .chain(self.checkpoints_extractor.get_sources())
            .chain(self.message_sender.get_sources())
            .chain(self.proof_composer.get_sources())
            .chain(self.tx_manager.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        suri: String,
        eth_api: PollingEthApi,
        beacon_client: BeaconClient,
        bridging_payment_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        mut api_provider: ApiProviderConnection,
        storage_path: String,
        genesis_time: u64,
        eth_unprocessed_block_storage_path: Option<String>,
        http_receiver: Option<UnboundedReceiver<EthTransaction>>,
    ) -> anyhow::Result<Self> {
        let block_storage: Arc<dyn UnprocessedBlockStorage> =
            if let Some(path) = eth_unprocessed_block_storage_path {
                Arc::new(ethereum::block_storage::JSONBlockStorage::new(path.into()).await?)
            } else {
                Arc::new(ethereum::block_storage::NoStorage)
            };

        let from_eth_block = eth_api.finalized_block().await?.header.number;

        let ethereum_block_listener =
            EthereumBlockListener::new(eth_api.clone(), from_eth_block, block_storage);

        let gear_block_listener = GearBlockListener::new(
            api_provider.clone(),
            Arc::new(crate::message_relayer::common::gear::block_storage::NoStorage),
        );

        let storage = Arc::new(JSONStorage::new(storage_path));

        let tx_manager = TransactionManager::new(storage.clone());

        let message_paid_event_extractor = MessagePaidEventExtractor::new(
            eth_api.clone(),
            bridging_payment_address,
            storage.clone(),
            genesis_time,
        );

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

        let client = api_provider
            .gclient_client(&suri)
            .expect("failed to create gclient");

        let latest_checkpoint =
            super::get_latest_checkpoint(checkpoint_light_client_address, client).await;

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let message_sender = message_sender::MessageSender::new(
            vft_manager_address,
            route,
            historical_proxy_address,
            api_provider.clone(),
            suri.clone(),
        );

        let proof_composer = proof_composer::ProofComposer::new(
            api_provider,
            beacon_client,
            eth_api.clone(),
            historical_proxy_address,
            suri,
        );

        let (transaction_data_extractor, tx_events_sender, tx_events_receiver) =
            if let Some(http_receiver) = http_receiver {
                let (tx_events_sender, tx_events_receiver) = unbounded_channel();
                let extractor = TransactionDataExtractor::new(
                    eth_api,
                    genesis_time,
                    tx_events_sender.downgrade(),
                    http_receiver,
                );
                (
                    Some(extractor),
                    Some(tx_events_sender),
                    Some(tx_events_receiver),
                )
            } else {
                (None, None, None)
            };

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            message_paid_event_extractor,
            checkpoints_extractor,
            latest_checkpoint,

            message_sender,
            proof_composer,
            tx_manager,

            transaction_data_extractor,
            tx_events_sender,
            tx_events_receiver,

            storage,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.spawn();

        if let Err(err) = self.storage.load(&self.tx_manager).await {
            log::warn!("Failed to load transaction and block status from storage: {err:?}")
        }

        let message_paid_events = if let Some(sender) = self.tx_events_sender {
            self.message_paid_event_extractor
                .spawn_into(ethereum_blocks, sender);
            if let Some(extractor) = self.transaction_data_extractor {
                extractor.spawn();
            }
            self.tx_events_receiver
                .expect("tx events receiver must exist when HTTP relay is enabled")
        } else {
            self.message_paid_event_extractor.spawn(ethereum_blocks)
        };
        let checkpoints = self
            .checkpoints_extractor
            .run(gear_blocks, self.latest_checkpoint)
            .await;
        let proof_composer = self.proof_composer.run(checkpoints);
        let message_sender = self.message_sender.run();
        self.tx_manager
            .run(message_paid_events, proof_composer, message_sender)
            .await
    }
}
