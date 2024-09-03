use std::iter;

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use primitive_types::H256;

use utils_prometheus::MeteredService;

use crate::message_relayer::common::{
    ethereum_block_listener::EthereumBlockListener, paid_messages_filter::PaidMessagesFilter,
};

use super::common::{
    ethereum_message_sender::EthereumMessageSender, gear_block_listener::GearBlockListener,
    merkle_root_extractor::MerkleRootExtractor,
    message_paid_event_extractor::MessagePaidEventExtractor,
    message_queued_event_extractor::MessageQueuedEventExtractor,
};

pub struct MessageRelayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: EthereumMessageSender,
}

impl MeteredService for MessageRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.message_sent_listener.get_sources())
            .chain(self.message_paid_listener.get_sources())
            .chain(self.paid_messages_filter.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
    }
}

impl MessageRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        from_block: Option<u32>,
        bridging_payment_address: H256,
    ) -> anyhow::Result<Self> {
        let from_gear_block = if let Some(block) = from_block {
            block
        } else {
            let block = gear_api.latest_finalized_block().await?;
            gear_api.block_hash_to_number(block).await?
        };

        let from_eth_block = eth_api.block_number().await?;

        log::info!(
            "Starting gear event processing from block #{}",
            from_gear_block
        );
        log::info!("Starting ethereum listener from block #{}", from_eth_block);

        let gear_block_listener = GearBlockListener::new(gear_api.clone(), from_gear_block);

        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let message_sent_listener = MessageQueuedEventExtractor::new(gear_api.clone());

        let message_paid_listener =
            MessagePaidEventExtractor::new(gear_api.clone(), bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new();

        let merkle_root_listener = MerkleRootExtractor::new(eth_api.clone(), gear_api.clone());

        let message_sender = EthereumMessageSender::new(eth_api, gear_api);

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            message_sent_listener,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_extractor: merkle_root_listener,
            message_sender,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let [gear_blocks_0, gear_blocks_1] = self.gear_block_listener.run();
        let ethereum_blocks = self.ethereum_block_listener.run();

        let messages = self.message_sent_listener.run(gear_blocks_0);
        let paid_messages = self.message_paid_listener.run(gear_blocks_1);

        let filtered_messages = self.paid_messages_filter.run(messages, paid_messages);

        let merkle_roots = self.merkle_root_extractor.run(ethereum_blocks);

        log::info!("Starting message relayer");

        self.message_sender
            .run(filtered_messages, merkle_roots)
            .await;

        Ok(())
    }
}
