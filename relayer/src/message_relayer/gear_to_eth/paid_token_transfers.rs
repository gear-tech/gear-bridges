use std::iter;

use ethereum_client::EthApi;
use primitive_types::H256;
use utils_prometheus::MeteredService;

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
                message_paid_event_extractor::MessagePaidEventExtractor,
                message_queued_event_extractor::MessageQueuedEventExtractor,
            },
            paid_messages_filter::PaidMessagesFilter,
        },
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,

    proof_fetcher: MerkleProofFetcher,
    status_fetcher: StatusFetcher,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.message_sent_listener.get_sources())
            .chain(self.message_paid_listener.get_sources())
            .chain(self.paid_messages_filter.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
    }
}

impl Relayer {
    pub async fn new(
        eth_api: EthApi,
        bridging_payment_address: H256,
        api_provider: ApiProviderConnection,
        confirmations_merkle_root: u64,
        confirmations_status: u64,
    ) -> anyhow::Result<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone());

        let message_sent_listener = MessageQueuedEventExtractor::new(api_provider.clone());

        let message_paid_listener = MessagePaidEventExtractor::new(bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new();

        let merkle_root_extractor = MerkleRootExtractor::new(
            eth_api.clone(),
            api_provider.clone(),
            confirmations_merkle_root,
        );

        let message_sender = MessageSender::new(MAX_RETRIES, eth_api.clone());

        let proof_fetcher = MerkleProofFetcher::new(api_provider);
        let status_fetcher = StatusFetcher::new(eth_api, confirmations_status);

        Ok(Self {
            gear_block_listener,

            message_sent_listener,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_extractor,
            message_sender,

            proof_fetcher,
            status_fetcher,
        })
    }

    pub async fn run(self) {
        let [gear_blocks_0, gear_blocks_1] = self.gear_block_listener.run().await;

        let messages = self.message_sent_listener.run(gear_blocks_0).await;
        let paid_messages = self.message_paid_listener.run(gear_blocks_1).await;

        let filtered_messages = self.paid_messages_filter.run(messages, paid_messages).await;

        let merkle_roots = self.merkle_root_extractor.spawn();

        let accumulator = Accumulator::new();
        let channel_messages = accumulator.run(filtered_messages, merkle_roots).await;

        let channel_message_data = self.proof_fetcher.spawn(channel_messages);
        let channel_tx_data = self.status_fetcher.spawn();

        self.message_sender
            .spawn(channel_message_data, channel_tx_data);
    }
}
