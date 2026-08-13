use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator, merkle_root_extractor::MerkleRootExtractor,
            message_sender::MessageSender, status_fetcher::StatusFetcher,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            merkle_proof_fetcher::MerkleProofFetcher, message_data_extractor::MessageDataExtractor,
            message_paid_event_extractor::MessagePaidEventExtractor,
            message_queued_event_extractor::MessageQueuedEventExtractor,
        },
        paid_messages_filter::PaidMessagesFilter,
        web_request::Message,
        MessageInBlock,
    },
    gear_to_eth::{storage::JSONStorage, tx_manager::TransactionManager},
};
use anyhow::Result as AnyResult;
use ethereum_client::EthApi;
use gclient::ext::sp_runtime::AccountId32;
use gear_common::api_provider::ApiProviderConnection;
use primitive_types::H256;
use sails_rs::ActorId;
use std::{collections::HashSet, iter, path::Path, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    listener_message_queued: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,

    proof_fetcher: MerkleProofFetcher,
    status_fetcher: StatusFetcher,

    accumulator: Accumulator,
    message_data_extractor: MessageDataExtractor,
    message_queued_receiver: UnboundedReceiver<MessageInBlock>,
    message_receiver: UnboundedReceiver<MessageInBlock>,

    tx_manager: TransactionManager,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.listener_message_queued.get_sources())
            .chain(self.message_paid_listener.get_sources())
            .chain(self.paid_messages_filter.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
            .chain(self.accumulator.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        eth_api: EthApi,
        bridging_payment_address: H256,
        api_provider: ApiProviderConnection,
        confirmations_merkle_root: u64,
        confirmations_status: u64,
        excluded_from_fees: HashSet<AccountId32>,
        receiver: UnboundedReceiver<Message>,
        storage_path: impl AsRef<Path>,
        governance_admin: ActorId,
        governance_pauser: ActorId,
    ) -> AnyResult<Self> {
        let storage = Arc::new(JSONStorage::new(storage_path));
        let tx_manager = TransactionManager::new(storage.clone());
        if let Err(e) = tx_manager.load_from_storage().await {
            log::warn!("Failed to load transaction manager state: {e}");
        }

        let gear_block_listener = GearBlockListener::new(api_provider.clone(), storage.clone());

        let (message_queued_sender, message_queued_receiver) = mpsc::unbounded_channel();
        let listener_message_queued =
            MessageQueuedEventExtractor::new(api_provider.clone(), message_queued_sender, storage);

        let message_paid_listener = MessagePaidEventExtractor::new(bridging_payment_address);

        let (roots_sender, roots_receiver) = mpsc::unbounded_channel();
        let merkle_root_extractor = MerkleRootExtractor::new(
            eth_api.clone(),
            api_provider.clone(),
            confirmations_merkle_root,
            roots_sender.clone(),
        );

        let message_sender = MessageSender::new(eth_api.clone());

        let proof_fetcher = MerkleProofFetcher::new(api_provider.clone());
        let status_fetcher = StatusFetcher::new(eth_api.clone(), confirmations_status);

        let (messages_sender, messages_receiver) = mpsc::unbounded_channel();
        let message_data_extractor =
            MessageDataExtractor::new(api_provider.clone(), messages_sender.downgrade(), receiver);
        let paid_messages_filter = PaidMessagesFilter::new(excluded_from_fees, messages_sender);

        let accumulator = Accumulator::new(
            roots_receiver,
            tx_manager.merkle_roots.clone(),
            governance_admin,
            governance_pauser,
            eth_api.clone(),
        );

        Ok(Self {
            gear_block_listener,

            listener_message_queued,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_extractor,
            message_sender,

            proof_fetcher,
            status_fetcher,
            accumulator,
            message_queued_receiver,
            message_data_extractor,
            message_receiver: messages_receiver,

            tx_manager,
        })
    }

    pub async fn run(self) -> AnyResult<()> {
        let [gear_blocks_0, gear_blocks_1] = self.gear_block_listener.run().await;

        let message_paid_receiver = self.message_paid_listener.run(gear_blocks_1).await;
        self.listener_message_queued.spawn(gear_blocks_0);

        self.paid_messages_filter
            .spawn(self.message_queued_receiver, message_paid_receiver);
        self.merkle_root_extractor.spawn();
        let accumulator_io = self.accumulator.spawn();

        let proof_fetcher_io = self.proof_fetcher.spawn();
        let status_fetcher_io = self.status_fetcher.spawn();

        self.message_data_extractor.spawn();
        let message_sender_io = self.message_sender.spawn();

        self.tx_manager
            .run(
                accumulator_io,
                self.message_receiver,
                proof_fetcher_io,
                message_sender_io,
                status_fetcher_io,
            )
            .await
    }
}
