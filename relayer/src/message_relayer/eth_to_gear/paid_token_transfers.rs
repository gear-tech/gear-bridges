use super::{
    message_sender, proof_composer,
    storage::{InboundRuntimeIdentity, JSONStorage, Storage},
    tx_manager,
};
use crate::message_relayer::common::{
    ethereum::{
        self, block_listener::BlockListener as EthereumBlockListener,
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
        from_eth_block: u64,
        eth_unprocessed_block_storage_path: String,
        http_receiver: Option<UnboundedReceiver<EthTransaction>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            std::path::Path::new(&storage_path).join("state.json").try_exists()?
                == std::path::Path::new(&eth_unprocessed_block_storage_path).try_exists()?,
            "HOLD: inbound transaction/discovery journal pair is incomplete; reconcile the original deployment coverage"
        );

        let gear_block_listener = GearBlockListener::new(
            api_provider.clone(),
            Arc::new(crate::message_relayer::common::gear::block_storage::NoStorage),
        );

        let storage = Arc::new(JSONStorage::new(&storage_path));

        let tx_manager = TransactionManager::new(storage.clone());

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

        let client = api_provider
            .gclient_client(&suri)
            .expect("failed to create gclient");

        let gear_genesis_hash = crate::rpc::retry_gear(
            &mut api_provider,
            "inbound Gear genesis",
            |api| async move { api.block_number_to_hash(0).await },
        )
        .await?;
        let identity = InboundRuntimeIdentity {
            ethereum_chain_id: eth_api.chain_id().await?,
            ethereum_genesis_hash: eth_api.get_block(0).await?.header.hash.0.into(),
            ethereum_start_block: from_eth_block,
            erc20_manager_address: None,
            bridging_payment_address: Some(bridging_payment_address),
            gear_genesis_hash,
            vft_manager_address,
            checkpoint_light_client_address,
            historical_proxy_address,
            gear_sender: client.account_id().clone().into(),
        };
        let block_storage = Arc::new(
            ethereum::block_storage::JSONBlockStorage::new(
                eth_unprocessed_block_storage_path.into(),
                identity.clone(),
            )
            .await?,
        );
        let expected_genesis = alloy::primitives::B256::from(identity.ethereum_genesis_hash.0);
        storage.bind_runtime_identity(identity).await?;
        eth_api
            .enable_finality_archive(
                &std::path::Path::new(&storage_path).join("ethereum-finality"),
                expected_genesis,
            )
            .await?;
        let ethereum_block_listener =
            EthereumBlockListener::new(eth_api.clone(), block_storage.clone());
        let message_paid_event_extractor = MessagePaidEventExtractor::new(
            eth_api.clone(),
            bridging_payment_address,
            storage.clone(),
            genesis_time,
            block_storage,
        );

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
            None,
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
        self.storage.load(&self.tx_manager).await?;

        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.spawn();

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
