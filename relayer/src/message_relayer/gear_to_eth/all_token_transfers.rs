use crate::{
    common::MAX_RETRIES,
    message_relayer::{
        common::{
            ethereum::{
                accumulator::Accumulator, merkle_root_extractor::MerkleRootExtractor,
                message_sender::MessageSender, status_fetcher::StatusFetcher,
            },
            gear::{
                block_listener::BlockListener as GearBlockListener,
                merkle_proof_fetcher::MerkleProofFetcher,
                message_queued_event_extractor::MessageQueuedEventExtractor,
            },
            MessageInBlock,
        },
        eth_to_gear::api_provider::ApiProviderConnection,
        gear_to_eth::{storage::JSONStorage, tx_manager::TransactionManager},
    },
};
use ethereum_client::EthApi;
use std::{iter, path::PathBuf, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    listener_message_queued: MessageQueuedEventExtractor,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,

    proof_fetcher: MerkleProofFetcher,
    status_fetcher: StatusFetcher,
    accumulator: Accumulator,

    message_queued_receiver: UnboundedReceiver<MessageInBlock>,

    tx_manager: TransactionManager,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.listener_message_queued.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
            .chain(self.accumulator.get_sources())
    }
}

impl Relayer {
    pub async fn new(
        eth_api: EthApi,

        api_provider: ApiProviderConnection,
        confirmations_merkle_root: u64,

        confirmations_status: u64,

        storage_path: PathBuf,
    ) -> anyhow::Result<Self> {
        let storage = Arc::new(JSONStorage::new(storage_path));
        let tx_manager = TransactionManager::new(storage.clone());
        if let Err(e) = tx_manager.load_from_storage().await {
            log::warn!("Failed to load transaction manager state: {e}");
        }

        let gear_block_listener = GearBlockListener::new(api_provider.clone(), storage.clone());

        let (message_queued_sender, message_queued_receiver) = mpsc::unbounded_channel();
        let listener_message_queued = MessageQueuedEventExtractor::new(
            api_provider.clone(),
            message_queued_sender,
            storage.clone(),
        );

        let (roots_sender, roots_receiver) = mpsc::unbounded_channel();
        let merkle_root_extractor = MerkleRootExtractor::new(
            eth_api.clone(),
            api_provider.clone(),
            confirmations_merkle_root,
            roots_sender,
        );

        let accumulator = Accumulator::new(roots_receiver, tx_manager.merkle_roots.clone());

        let message_sender = MessageSender::new(MAX_RETRIES, eth_api.clone());

        let proof_fetcher = MerkleProofFetcher::new(api_provider);
        let status_fetcher = StatusFetcher::new(eth_api, confirmations_status);

        Ok(Self {
            gear_block_listener,

            listener_message_queued,

            merkle_root_extractor,
            message_sender,

            proof_fetcher,
            status_fetcher,
            accumulator,

            message_queued_receiver,

            tx_manager,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;

        self.listener_message_queued.spawn(gear_blocks);
        self.merkle_root_extractor.spawn();

        let accumulator_io = self.accumulator.spawn();
        let proof_fetcher_io = self.proof_fetcher.spawn();
        let status_fetcher_io = self.status_fetcher.spawn();

        let message_sender_io = self.message_sender.spawn();

        if let Err(err) = self
            .tx_manager
            .run(
                accumulator_io,
                self.message_queued_receiver,
                proof_fetcher_io,
                message_sender_io,
                status_fetcher_io,
            )
            .await
        {
            log::error!("Transaction manager exited with error: {err:?}");
        }
    }
}
