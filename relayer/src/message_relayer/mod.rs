use ethereum_client::EthApi;
use gear_rpc_client::{dto::Message, GearApi};
use primitive_types::H256;

use utils_prometheus::MeteredService;

mod common;

use common::{
    block_listener::BlockListener, merkle_root_listener::MerkleRootListener,
    message_queued_listener::MessageQueuedListener, message_sender::MessageSender,
};

type AuthoritySetId = u64;

struct MessageInBlock {
    message: Message,
    block: u32,
    block_hash: H256,
}

pub struct MessageRelayer {
    block_listener: BlockListener,
    event_listener: MessageQueuedListener,
    merkle_root_listener: MerkleRootListener,
    message_sender: MessageSender,
}

impl MeteredService for MessageRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.event_listener
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
        _bridging_payment_address: Option<H256>,
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

        let block_listener = BlockListener::new(gear_api.clone(), from_gear_block);

        let event_listener = MessageQueuedListener::new(gear_api.clone());

        let merkle_root_listener =
            MerkleRootListener::new(eth_api.clone(), gear_api.clone(), from_eth_block);

        let message_sender = MessageSender::new(eth_api, gear_api);

        Ok(Self {
            block_listener,
            event_listener,
            merkle_root_listener,
            message_sender,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let block_listener = self.block_listener.run();
        let messages = self.event_listener.run(block_listener);
        let merkle_roots = self.merkle_root_listener.run();

        log::info!("Starting message relayer");

        self.message_sender.run(messages, merkle_roots).await;

        Ok(())
    }
}
