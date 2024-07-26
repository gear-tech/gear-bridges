use ethereum_client::EthApi;
use gear_rpc_client::{dto::Message, GearApi};
use primitive_types::{H256, U256};

use utils_prometheus::MeteredService;

mod event_listener;
mod merkle_root_listener;
mod message_processor;

use event_listener::EventListener;
use merkle_root_listener::MerkleRootListener;
use message_processor::MessageProcessor;

type AuthoritySetId = u64;
type BlockNumber = u32;

enum BlockEvent {
    MessageSent { message: MessageInBlock },
    MessagePaid { nonce: U256 },
}

struct MessageInBlock {
    message: Message,
    block: u32,
    block_hash: H256,
}

#[derive(Clone, Copy)]
struct RelayedMerkleRoot {
    gear_block: u32,
    authority_set_id: AuthoritySetId,
}

pub struct MessageRelayer {
    event_processor: EventListener,
    merkle_root_listener: MerkleRootListener,
    message_processor: MessageProcessor,
}

impl MeteredService for MessageRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.event_processor
            .get_sources()
            .into_iter()
            .chain(self.merkle_root_listener.get_sources())
            .chain(self.message_processor.get_sources())
    }
}

impl MessageRelayer {
    pub async fn new(
        gear_api: GearApi,
        eth_api: EthApi,
        from_block: Option<u32>,
        bridging_payment_address: Option<H256>,
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

        let event_processor =
            EventListener::new(gear_api.clone(), from_gear_block, bridging_payment_address);

        let merkle_root_listener =
            MerkleRootListener::new(eth_api.clone(), gear_api.clone(), from_eth_block);

        let message_processor = MessageProcessor::new(eth_api, gear_api);

        Ok(Self {
            event_processor,
            merkle_root_listener,
            message_processor,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let messages = self.event_processor.run();
        let merkle_roots = self.merkle_root_listener.run();

        log::info!("Starting message relayer");
        self.message_processor.run(messages, merkle_roots).await;

        Ok(())
    }
}
