use crate::message_relayer::{
    common::{
        self, web_request::Message, AuthoritySetId, GearBlock, GearBlockNumber, MessageInBlock,
    },
    eth_to_gear::api_provider::ApiProviderConnection,
};
use anyhow::Result as AnyResult;
use ethereum_common::U256;
use std::{cmp::Ordering, ops::Deref};
use tokio::{
    sync::mpsc::{UnboundedReceiver, WeakUnboundedSender},
    task,
};

pub type BlockData = (GearBlock, AuthoritySetId);

/// The purpose of the entity is to receive requests to relay message(s) from an external source,
/// than get required data for relaying the message(s) and send the data further.
///
/// An example of an external requester is HTTP server.
pub struct MessageDataExtractor {
    api_provider: ApiProviderConnection,
    sender: WeakUnboundedSender<MessageInBlock>,
    receiver: UnboundedReceiver<Message>,
    blocks: BlockDataList,
}

impl MessageDataExtractor {
    pub fn new(
        api_provider: ApiProviderConnection,
        sender: WeakUnboundedSender<MessageInBlock>,
        receiver: UnboundedReceiver<Message>,
    ) -> Self {
        Self {
            api_provider,
            sender,
            receiver,
            blocks: BlockDataList::new(1_000),
        }
    }

    pub fn spawn(self) {
        task::spawn(self::task(self));
    }

    async fn run_inner(&mut self) -> anyhow::Result<()> {
        loop {
            let Some(message) = self.receiver.recv().await else {
                return Ok(());
            };

            log::trace!(r#"Processing message: "{message:?}""#);

            let block_data = match self.find_block_data(message.block) {
                Some(block_data) => block_data,
                None => self.retreive_block_data(message.block).await?,
            };

            log::trace!(r#"Found data for the message block: "{block_data:?}""#);

            let (block, authority_set_id) = block_data;
            let messages = common::message_queued_events_of(&block);
            let block_hash = block.hash();
            let Some(sender) = self.sender.upgrade() else {
                log::info!("Unable to upgrade sender channel.");

                return Ok(());
            };

            for message_queued in messages {
                if U256::from_big_endian(&message_queued.nonce_be).0 != message.nonce.0 {
                    log::info!("Message nonce mismatch, skipping {message_queued:?}");
                    continue;
                }

                log::trace!("Processing message in block: {message_queued:?}");

                if sender
                    .send(MessageInBlock {
                        message: message_queued,
                        block: GearBlockNumber(block.number()),
                        block_hash: block_hash.0.into(),
                        authority_set_id,
                    })
                    .is_err()
                {
                    log::info!("Sender channel closed.");
                    return Ok(());
                }

                break;
            }
        }
    }

    fn find_block_data(&self, block_number: u32) -> Option<BlockData> {
        self.blocks.find_by_block_number(block_number).cloned()
    }

    async fn retreive_block_data(&mut self, block_number: u32) -> AnyResult<BlockData> {
        let gear_api = self.api_provider.client();

        let block_hash = gear_api.block_number_to_hash(block_number).await?;
        let block = gear_api.get_block_at(block_hash).await?;
        let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

        let header = block.header().clone();
        let events = gear_api.get_events_at(Some(block_hash)).await?;

        let justification = gear_api.get_justification(block_hash).await?;

        let block_data = (
            GearBlock::new(header, events, justification),
            AuthoritySetId(authority_set_id),
        );
        self.blocks.push(block_data.clone());

        Ok(block_data)
    }
}

async fn task(mut this: MessageDataExtractor) {
    loop {
        let result = this.run_inner().await;
        let Err(e) = result else {
            log::trace!("Message data extractor exiting...");
            return;
        };

        log::error!("Message data extractor failed: {e}");

        match this.api_provider.reconnect().await {
            Ok(_) => {
                log::info!("Message queued extractor reconnected");
            }

            Err(e) => {
                log::error!("Message queued extractor unable to reconnect: {e:?}");
                return;
            }
        }
    }
}

struct BlockDataList(Vec<BlockData>);

impl BlockDataList {
    fn compare(block_data: &BlockData, block_number: u32) -> Ordering {
        let (block, _authority_set_id) = block_data;

        block_number.cmp(&block.number())
    }

    pub fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub fn push(&mut self, block_data_new: BlockData) {
        // remove the oldest block
        if self.len() >= self.0.capacity() {
            self.0.pop();
        }

        let Err(i) = self
            .binary_search_by(|block_data| Self::compare(block_data, block_data_new.0.number()))
        else {
            return;
        };

        self.0.insert(i, block_data_new);
    }

    pub fn find_by_block_number(&self, block_number: u32) -> Option<&BlockData> {
        let Ok(i) = self.binary_search_by(|block_data| Self::compare(block_data, block_number))
        else {
            return None;
        };

        self.get(i)
    }
}

impl Deref for BlockDataList {
    type Target = [BlockData];

    fn deref(&self) -> &Self::Target {
        &self.0[..]
    }
}
