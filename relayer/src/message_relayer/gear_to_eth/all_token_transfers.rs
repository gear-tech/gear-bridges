use std::iter;

use ethereum_client::EthApi;
use utils_prometheus::MeteredService;

use crate::{
    common::MAX_RETRIES,
    message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator,
            merkle_root_extractor::MerkleRootExtractor, message_sender::MessageSender,
            status_fetcher::StatusFetcher,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            message_queued_event_extractor::MessageQueuedEventExtractor,
            merkle_proof_fetcher::MerkleProofFetcher,
        },
    },
    eth_to_gear::api_provider::ApiProviderConnection,
}};

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,

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
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
    }
}

impl Relayer {
    pub async fn new(
        eth_api: EthApi,
        from_block: Option<u32>,
        api_provider: ApiProviderConnection,
        confirmations_merkle_root: u64,
        
    confirmations_status: u64,
    ) -> anyhow::Result<Self> {
        let from_gear_block = if let Some(block) = from_block {
            block
        } else {
            let gear_api = api_provider.client();
            let block = gear_api.latest_finalized_block().await?;
            gear_api.block_hash_to_number(block).await?
        };

        let gear_block_listener = GearBlockListener::new(api_provider.clone(), from_gear_block);

        let message_sent_listener = MessageQueuedEventExtractor::new(api_provider.clone());

        let merkle_root_extractor =
            MerkleRootExtractor::new(eth_api.clone(), api_provider.clone(), confirmations_merkle_root);

        let message_sender = MessageSender::new(MAX_RETRIES, eth_api.clone());

        let proof_fetcher = MerkleProofFetcher::new(api_provider);
        let status_fetcher = StatusFetcher::new(eth_api, confirmations_status);

        Ok(Self {
            gear_block_listener,

            message_sent_listener,

            merkle_root_extractor,
            message_sender,

            proof_fetcher,
            status_fetcher,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;

        let messages = self.message_sent_listener.run(gear_blocks).await;

        let merkle_roots = self.merkle_root_extractor.spawn();
        let accumulator = Accumulator::new();
        let channel_messages = accumulator.run(messages, merkle_roots).await;

        let channel_message_data = self.proof_fetcher.spawn(channel_messages);
        let channel_tx_data = self.status_fetcher.spawn();

        self.message_sender.spawn(channel_message_data, channel_tx_data);
    }
}
