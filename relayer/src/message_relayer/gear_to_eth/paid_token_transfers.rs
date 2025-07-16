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
                message_data_extractor::MessageDataExtractor,
                message_paid_event_extractor::MessagePaidEventExtractor,
                message_queued_event_extractor::MessageQueuedEventExtractor,
            },
            paid_messages_filter::PaidMessagesFilter,
            web_request::Message,
            AuthoritySetId, GearBlockNumber, MessageInBlock, RelayedMerkleRoot,
        },
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use anyhow::Result as AnyResult;
use ethereum_client::EthApi;
use gclient::ext::sp_runtime::AccountId32;
use primitive_types::H256;
use std::{collections::HashSet, iter};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task, time,
};
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,

    listener_message_queued: MessageQueuedEventExtractor,
    message_paid_listener: MessagePaidEventExtractor,

    paid_messages_filter: PaidMessagesFilter,

    merkle_root_extractor: MerkleRootExtractor,
    message_sender: MessageSender,

    proof_fetcher: MerkleProofFetcher,
    status_fetcher: StatusFetcher,

    accumulator: Accumulator,
    message_data_extractor: MessageDataExtractor,
    message_queued_receiver: UnboundedReceiver<MessageInBlock>,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.listener_message_queued.get_sources())
            .chain(self.message_paid_listener.get_sources())
            .chain(self.paid_messages_filter.get_sources())
            .chain(self.merkle_root_extractor.get_sources())
            .chain(self.message_sender.get_sources())
            .chain(self.accumulator.get_sources())
    }
}

impl Relayer {
    pub async fn new(
        eth_api: EthApi,
        bridging_payment_address: H256,
        api_provider: ApiProviderConnection,
        confirmations_merkle_root: u64,
        confirmations_status: u64,
        excluded_from_fees: HashSet<AccountId32>,
        receiver: UnboundedReceiver<Message>,
    ) -> AnyResult<Self> {
        let gear_block_listener = GearBlockListener::new(api_provider.clone());

        let (message_queued_sender, message_queued_receiver) = mpsc::unbounded_channel();
        let listener_message_queued =
            MessageQueuedEventExtractor::new(api_provider.clone(), message_queued_sender);

        let message_paid_listener = MessagePaidEventExtractor::new(bridging_payment_address);

        let paid_messages_filter = PaidMessagesFilter::new(excluded_from_fees);

        let (roots_sender, roots_receiver) = mpsc::unbounded_channel();
        let merkle_root_extractor = MerkleRootExtractor::new(
            eth_api.clone(),
            api_provider.clone(),
            confirmations_merkle_root,
            roots_sender.clone(),
        );

        let message_sender = MessageSender::new(MAX_RETRIES, eth_api.clone());

        let proof_fetcher = MerkleProofFetcher::new(api_provider.clone());
        let status_fetcher = StatusFetcher::new(eth_api.clone(), confirmations_status);

        let (messages_sender, messages_receiver) = mpsc::unbounded_channel();
        let accumulator = Accumulator::new(roots_receiver, messages_receiver);

        let message_data_extractor =
            MessageDataExtractor::new(api_provider.clone(), messages_sender, receiver);

        task::spawn(self::fetch_merkle_roots(
            eth_api,
            api_provider,
            roots_sender,
        ));

        Ok(Self {
            gear_block_listener,

            listener_message_queued,
            message_paid_listener,

            paid_messages_filter,

            merkle_root_extractor,
            message_sender,

            proof_fetcher,
            status_fetcher,
            accumulator,
            message_queued_receiver,
            message_data_extractor,
        })
    }

    pub async fn run(self) {
        let [gear_blocks_0, gear_blocks_1] = self.gear_block_listener.run().await;

        let message_paid_receiver = self.message_paid_listener.run(gear_blocks_1).await;
        self.listener_message_queued.spawn(gear_blocks_0);

        self.paid_messages_filter.spawn(
            self.message_queued_receiver,
            message_paid_receiver,
            self.message_data_extractor.sender().clone(),
        );
        self.merkle_root_extractor.spawn();
        let channel_messages = self.accumulator.spawn();

        let channel_message_data = self.proof_fetcher.spawn(channel_messages);
        let channel_tx_data = self.status_fetcher.spawn();

        self.message_data_extractor.spawn();

        self.message_sender
            .spawn(channel_message_data, channel_tx_data);
    }
}

async fn fetch_merkle_roots(
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    sender: UnboundedSender<RelayedMerkleRoot>,
) {
    if let Err(e) = fetch_merkle_roots_inner(eth_api, api_provider, sender).await {
        log::error!("Task fetch_merkle_roots failed: {e:?}");
    }
}

async fn fetch_merkle_roots_inner(
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    sender: UnboundedSender<RelayedMerkleRoot>,
) -> AnyResult<()> {
    const COUNT: u64 = 2_000;

    let block_finalized = eth_api.finalized_block_number().await?;
    let gear_api = api_provider.client();

    for i in 0..50 {
        let block_range = crate::common::create_range(
            (block_finalized - (i + 1) * COUNT).into(),
            block_finalized - i * COUNT,
        );
        let merkle_roots = eth_api
            .fetch_merkle_roots_in_range(block_range.from, block_range.to)
            .await?;

        let len = merkle_roots.len();
        log::trace!("Found {len} entry(ies) with merkle roots (i = {i})");
        for (root, _block_number_eth) in merkle_roots {
            let block_hash = gear_api
                .block_number_to_hash(root.block_number as u32)
                .await?;
            let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

            sender.send(RelayedMerkleRoot {
                block: GearBlockNumber(root.block_number as u32),
                block_hash,
                authority_set_id: AuthoritySetId(authority_set_id),
                merkle_root: root.merkle_root,
            })?;
        }

        log::trace!("Successfuly sent {len} merkle root entry(ies) (i = {i})");

        time::sleep(time::Duration::from_secs(5)).await;
    }

    Ok(())
}
