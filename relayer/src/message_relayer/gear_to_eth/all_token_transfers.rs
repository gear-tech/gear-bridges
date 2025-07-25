use crate::{
    common::MAX_RETRIES,
    message_relayer::common::{
        ethereum::{
            accumulator::Accumulator, merkle_root_extractor::MerkleRootExtractor,
            message_sender::MessageSender, status_fetcher::StatusFetcher,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            merkle_proof_fetcher::MerkleProofFetcher,
            message_queued_event_extractor::MessageQueuedEventExtractor,
        },
    },
};
use ethereum_client::EthApi;
use gear_common::ApiProviderConnection;
use std::{iter, sync::Arc};
use tokio::sync::mpsc;
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    listener_message_queued: MessageQueuedEventExtractor,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,

    proof_fetcher: MerkleProofFetcher,
    status_fetcher: StatusFetcher,
    accumulator: Accumulator,
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
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(
            api_provider.clone(),
            Arc::new(crate::message_relayer::common::gear::block_storage::NoStorage),
        );

        let (message_queued_sender, message_queued_receiver) = mpsc::unbounded_channel();
        let listener_message_queued =
            MessageQueuedEventExtractor::new(api_provider.clone(), message_queued_sender);

        let (roots_sender, roots_receiver) = mpsc::unbounded_channel();
        let merkle_root_extractor = MerkleRootExtractor::new(
            eth_api.clone(),
            api_provider.clone(),
            confirmations_merkle_root,
            roots_sender,
        );

        let accumulator = Accumulator::new(roots_receiver, message_queued_receiver);

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
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;

        self.listener_message_queued.spawn(gear_blocks);
        self.merkle_root_extractor.spawn();
        let channel_messages = self.accumulator.spawn();

        let channel_message_data = self.proof_fetcher.spawn(channel_messages);
        let channel_tx_data = self.status_fetcher.spawn();

        self.message_sender
            .spawn(channel_message_data, channel_tx_data);
    }
}
