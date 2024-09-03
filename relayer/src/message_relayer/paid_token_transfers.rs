use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use primitive_types::H256;

use utils_prometheus::MeteredService;

use crate::message_relayer::common::paid_messages_filter::PaidMessagesFilter;

use super::common::{
    ethereum_message_sender::EthereumMessageSender, gear_block_listener::GearBlockListener,
    merkle_root_listener::MerkleRootListener,
    message_paid_event_extractor::MessagePaidEventExtractor,
    message_queued_event_extractor::MessageQueuedEventExtractor,
};

pub struct MessageRelayer {
    block_listener: GearBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_listener: MerkleRootListener,
    message_sender: EthereumMessageSender,
}

impl MeteredService for MessageRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.message_sent_listener
            .get_sources()
            .into_iter()
            .chain(self.merkle_root_listener.get_sources())
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

        let block_listener = GearBlockListener::new(gear_api.clone(), from_gear_block);

        let message_sent_listener = MessageQueuedEventExtractor::new(gear_api.clone());

        let message_paid_listener =
            MessagePaidEventExtractor::new(gear_api.clone(), bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new();

        let merkle_root_listener =
            MerkleRootListener::new(eth_api.clone(), gear_api.clone(), from_eth_block);

        let message_sender = EthereumMessageSender::new(eth_api, gear_api);

        Ok(Self {
            block_listener,

            message_sent_listener,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_listener,
            message_sender,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let [blocks_0, blocks_1] = self.block_listener.run();

        let messages = self.message_sent_listener.run(blocks_0);
        let paid_messages = self.message_paid_listener.run(blocks_1);

        let filtered_messages = self.paid_messages_filter.run(messages, paid_messages);

        let merkle_roots = self.merkle_root_listener.run();

        log::info!("Starting message relayer");

        self.message_sender
            .run(filtered_messages, merkle_roots)
            .await;

        Ok(())
    }
}
