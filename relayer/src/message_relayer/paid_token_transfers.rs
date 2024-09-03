use std::{
    collections::HashMap,
    sync::mpsc::{channel, Receiver, Sender},
};

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use primitive_types::H256;

use utils_prometheus::MeteredService;

use super::common::{
    block_listener::BlockListener,
    merkle_root_listener::MerkleRootListener,
    message_paid_listener::{MessagePaidListener, PaidMessage},
    message_queued_listener::MessageQueuedListener,
    message_sender::MessageSender,
    MessageInBlock,
};

pub struct MessageRelayer {
    block_listener: BlockListener,

    message_sent_listener: MessageQueuedListener,
    message_paid_listener: MessagePaidListener,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_listener: MerkleRootListener,
    message_sender: MessageSender,
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

        let block_listener = BlockListener::new(gear_api.clone(), from_gear_block);

        let message_sent_listener = MessageQueuedListener::new(gear_api.clone());

        let message_paid_listener =
            MessagePaidListener::new(gear_api.clone(), bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new();

        let merkle_root_listener =
            MerkleRootListener::new(eth_api.clone(), gear_api.clone(), from_eth_block);

        let message_sender = MessageSender::new(eth_api, gear_api);

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

struct PaidMessagesFilter {
    pending_messages: HashMap<[u8; 32], MessageInBlock>,
    pending_nonces: Vec<[u8; 32]>,
}

impl PaidMessagesFilter {
    fn new() -> Self {
        Self {
            pending_messages: HashMap::default(),
            pending_nonces: vec![],
        }
    }

    fn run(
        mut self,
        messages: Receiver<MessageInBlock>,
        paid_messages: Receiver<PaidMessage>,
    ) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &messages, &paid_messages);
                if let Err(err) = res {
                    log::error!("Paid messages filter failed: {}", err);
                }
            }
        });

        receiver
    }

    fn run_inner(
        &mut self,
        sender: &Sender<MessageInBlock>,
        messages: &Receiver<MessageInBlock>,
        paid_messages: &Receiver<PaidMessage>,
    ) -> anyhow::Result<()> {
        loop {
            for message in messages.try_iter() {
                self.pending_messages
                    .insert(message.message.nonce_le, message)
                    .expect("Received 2 messages with the same nonce");
            }

            for PaidMessage { nonce } in paid_messages.try_iter() {
                self.pending_nonces.push(nonce);
            }

            for i in (0..self.pending_nonces.len()).rev() {
                if let Some(message) = self.pending_messages.remove(&self.pending_nonces[i]) {
                    sender.send(message)?;
                    self.pending_nonces.remove(i);
                }
            }
        }
    }
}
