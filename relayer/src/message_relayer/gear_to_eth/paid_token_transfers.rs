use std::iter;

use ethereum_client::EthApi;
use primitive_types::H256;
use utils_prometheus::MeteredService;

use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::Accumulator, merkle_root_extractor::MerkleRootExtractor,
            message_sender::MessageSender,
        },
        gear::{
            block_listener::BlockListener as GearBlockListener,
            message_paid_event_extractor::MessagePaidEventExtractor,
            message_queued_event_extractor::MessageQueuedEventExtractor,
        },
        paid_messages_filter::PaidMessagesFilter,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
};

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,
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
        from_block: Option<u32>,
        bridging_payment_address: H256,
        api_provider: ApiProviderConnection,
        confirmations: u64,
    ) -> anyhow::Result<Self> {
        let from_gear_block = if let Some(block) = from_block {
            block
        } else {
            let gear_api = api_provider.client();
            let block = gear_api.latest_finalized_block().await?;
            gear_api.block_hash_to_number(block).await?
        };

        log::info!("Starting gear event processing from block #{from_gear_block}");

        let gear_block_listener = GearBlockListener::new(api_provider.clone(), from_gear_block);

        let message_sent_listener = MessageQueuedEventExtractor::new(api_provider.clone());

        let message_paid_listener =
            MessagePaidEventExtractor::new(api_provider.clone(), bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new();

        let merkle_root_extractor =
            MerkleRootExtractor::new(eth_api.clone(), api_provider.clone(), confirmations);

        let message_sender = MessageSender::new(eth_api, api_provider);

        Ok(Self {
            gear_block_listener,

            message_sent_listener,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_extractor,
            message_sender,
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

        self.message_sender.run(channel_messages).await;
    }
}
