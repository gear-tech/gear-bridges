use std::iter;

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use utils_prometheus::MeteredService;

use crate::message_relayer::common::{
    ethereum::{
        block_listener::BlockListener as EthereumBlockListener,
        merkle_root_extractor::MerkleRootExtractor, message_sender::MessageSender,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener,
        message_queued_event_extractor::MessageQueuedEventExtractor,
    },
    GSdkArgs,
};

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    message_sent_listener: MessageQueuedEventExtractor,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.message_sent_listener.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
    }
}

impl Relayer {
    pub async fn new(
        gear_api: GearApi,
        args: GSdkArgs,
        eth_api: EthApi,
        from_block: Option<u32>,
    ) -> anyhow::Result<Self> {
        let from_gear_block = if let Some(block) = from_block {
            block
        } else {
            let block = gear_api.latest_finalized_block().await?;
            gear_api.block_hash_to_number(block).await?
        };

        let from_eth_block = eth_api.finalized_block_number().await?;

        let gear_block_listener = GearBlockListener::new(args, from_gear_block);

        let ethereum_block_listener = EthereumBlockListener::new(eth_api.clone(), from_eth_block);

        let message_sent_listener = MessageQueuedEventExtractor::new(gear_api.clone());

        let merkle_root_listener = MerkleRootExtractor::new(eth_api.clone(), gear_api.clone());

        let message_sender = MessageSender::new(eth_api, gear_api);

        Ok(Self {
            gear_block_listener,
            ethereum_block_listener,

            message_sent_listener,

            merkle_root_extractor: merkle_root_listener,
            message_sender,
        })
    }

    pub async fn run(self) {
        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.run().await;

        let messages = self.message_sent_listener.run(gear_blocks).await;

        let merkle_roots = self.merkle_root_extractor.run(ethereum_blocks).await;

        self.message_sender.run(messages, merkle_roots).await;
    }
}
