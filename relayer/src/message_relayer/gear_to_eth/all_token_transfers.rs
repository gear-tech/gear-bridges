use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator, merkle_root_extractor::MerkleRootExtractor,
            message_sender::MessageSender, status_fetcher::StatusFetcher,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            merkle_proof_fetcher::MerkleProofFetcher,
            message_queued_event_extractor::{bind_event_storage, MessageQueuedEventExtractor},
        },
        MessageInBlock,
    },
    gear_to_eth::{
        storage::{GearEventStream, JSONStorage, OutboundLaneIdentity, Storage},
        tx_manager::TransactionManager,
    },
};
use alloy::providers::Provider;
use ethereum_client::EthApi;
use gear_common::api_provider::ApiProviderConnection;
use sails_rs::ActorId;
use std::{iter, path::Path, sync::Arc};
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

        mut api_provider: ApiProviderConnection,
        from_block: Option<u32>,
        confirmations_merkle_root: u64,

        confirmations_status: u64,

        storage_path: impl AsRef<Path>,

        governance: (ActorId, ActorId),
    ) -> anyhow::Result<Self> {
        let (governance_admin, governance_pauser) = governance;
        let storage_path = storage_path.as_ref();
        let destination_genesis = ethereum_client::get_block(eth_api.raw_provider(), 0)
            .await?
            .header
            .hash;
        let storage = Arc::new(JSONStorage::new(storage_path));
        storage
            .bind_outbound_lane(OutboundLaneIdentity {
                destination_chain_id: eth_api.raw_provider().get_chain_id().await?,
                destination_genesis_hash: destination_genesis.0.into(),
                message_queue_address: eth_api.message_queue_address(),
                bridging_payment_address: None,
                fee_exempt_sources: Default::default(),
                sender_address: eth_api.sender_address(),
            })
            .await?;
        let event_start_block =
            bind_event_storage(&mut api_provider, storage.as_ref(), from_block).await?;
        let replay_start_block = storage
            .replay_from_block(event_start_block, GearEventStream::Queued)
            .await?;
        let tx_manager = TransactionManager::new(storage.clone());
        tx_manager.load_from_storage().await?;
        eth_api
            .enable_finality_archive(&storage_path.join("ethereum-finality"), destination_genesis)
            .await?;
        tx_manager.verify_completed(&eth_api).await?;

        let gear_block_listener = GearBlockListener::new(api_provider.clone(), storage.clone())
            .start_from(Some(replay_start_block));
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
            storage.clone(),
        );

        let accumulator = Accumulator::new(
            roots_receiver,
            tx_manager.merkle_roots.clone(),
            storage.clone(),
            governance_admin,
            governance_pauser,
            eth_api.clone(),
        );

        let message_sender = MessageSender::new(eth_api.clone());

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

    pub async fn run(self) -> anyhow::Result<()> {
        let [gear_blocks] = self.gear_block_listener.run().await;

        self.listener_message_queued.spawn(gear_blocks);
        self.merkle_root_extractor.spawn();

        let accumulator_io = self.accumulator.spawn();
        let proof_fetcher_io = self.proof_fetcher.spawn();
        let status_fetcher_io = self.status_fetcher.spawn();

        let message_sender_io = self.message_sender.spawn();

        self.tx_manager
            .run(
                accumulator_io,
                self.message_queued_receiver,
                proof_fetcher_io,
                message_sender_io,
                status_fetcher_io,
            )
            .await
    }
}
