use std::{
    collections::{btree_map::Entry, BTreeMap, HashSet},
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::Duration,
};

use bridging_payment::UserReply as BridgingPaymentUserReply;
use ethereum_client::{Contracts as EthApi, TxHash, TxStatus};
use gear_rpc_client::{dto::Message, GearApi};
use keccak_hash::keccak_256;
use parity_scale_codec::Decode;
use primitive_types::{H256, U256};

use crate::metrics::{impl_metered_service, MeteredService};

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);
const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

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
}

pub struct MessageRelayer {
    gear_api: GearApi,
    eth_api: EthApi,

    metrics: Metrics,
}

impl_metered_service! {
    struct Metrics {
        //
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {})
    }
}

impl MeteredService for MessageRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MessageRelayer {
    pub fn new(gear_api: GearApi, eth_api: EthApi) -> Self {
        let metrics = Metrics::new();

        Self {
            gear_api,
            eth_api,
            metrics,
        }
    }

    pub async fn run(
        self,
        from_block: Option<u32>,
        bridging_payment_address: Option<H256>,
    ) -> anyhow::Result<()> {
        let from_gear_block = if let Some(block) = from_block {
            block
        } else {
            let block = self.gear_api.latest_finalized_block().await?;
            self.gear_api.block_hash_to_number(block).await?
        };

        let from_eth_block = self.eth_api.block_number().await?;

        log::info!(
            "Starting gear event processing from block #{}",
            from_gear_block
        );
        let messages = EventProcessor::new(
            self.gear_api.clone(),
            from_gear_block,
            bridging_payment_address,
        )
        .run();

        log::info!("Starting ethereum listener from block #{}", from_eth_block);
        let merkle_roots =
            MerkleRootListener::new(self.eth_api.clone(), self.gear_api.clone(), from_eth_block)
                .run();

        log::info!("Starting message relayer");
        MessageProcessor::new(self.eth_api, self.gear_api, messages, merkle_roots)
            .run()
            .await;

        Ok(())
    }
}

struct EventProcessor {
    gear_api: GearApi,
    from_block: u32,
    bridging_payment_address: Option<H256>,
}

impl EventProcessor {
    fn new(gear_api: GearApi, from_block: u32, bridging_payment_address: Option<H256>) -> Self {
        Self {
            gear_api,
            from_block,
            bridging_payment_address,
        }
    }

    fn run(self) -> Receiver<BlockEvent> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    log::error!("Event processor failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(&self, sender: &Sender<BlockEvent>) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        loop {
            let finalized_head = self.gear_api.latest_finalized_block().await?;
            let finalized_head = self.gear_api.block_hash_to_number(finalized_head).await?;

            if finalized_head >= current_block {
                for block in current_block..=finalized_head {
                    self.process_block_events(block, sender).await?;
                }

                current_block = finalized_head + 1;
            } else {
                thread::sleep(GEAR_BLOCK_TIME_APPROX);
            }
        }
    }

    async fn process_block_events(
        &self,
        block: u32,
        sender: &Sender<BlockEvent>,
    ) -> anyhow::Result<()> {
        log::info!("Processing gear block #{}", block);
        let block_hash = self.gear_api.block_number_to_hash(block).await?;

        let messages = self.gear_api.message_queued_events(block_hash).await?;
        if !messages.is_empty() {
            log::info!("Found {} sent messages", messages.len());

            for message in messages {
                sender.send(BlockEvent::MessageSent {
                    message: MessageInBlock {
                        message,
                        block,
                        block_hash,
                    },
                })?;
            }
        }

        if let Some(bridging_payment_address) = self.bridging_payment_address {
            let messages = self
                .gear_api
                .user_message_sent_events(bridging_payment_address, block_hash)
                .await?;
            if !messages.is_empty() {
                log::info!("Found {} paid messages", messages.len());

                for message in messages {
                    let user_reply = BridgingPaymentUserReply::decode(&mut &message.payload[..])?;
                    sender.send(BlockEvent::MessagePaid {
                        nonce: user_reply.nonce,
                    })?;
                }
            }
        }

        Ok(())
    }
}

struct MerkleRootListener {
    eth_api: EthApi,
    gear_api: GearApi,
    from_block: u64,
}

impl MerkleRootListener {
    fn new(eth_api: EthApi, gear_api: GearApi, from_block: u64) -> Self {
        Self {
            eth_api,
            gear_api,
            from_block,
        }
    }

    fn run(self) -> Receiver<(AuthoritySetId, RelayedMerkleRoot)> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender).await;
                if let Err(err) = res {
                    log::error!("Merkle root listener failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &self,
        sender: &Sender<(AuthoritySetId, RelayedMerkleRoot)>,
    ) -> anyhow::Result<()> {
        let mut current_block = self.from_block;

        loop {
            let latest = self.eth_api.block_number().await?;
            if latest >= current_block {
                log::info!("Processing ethereum blocks #{}..#{}", current_block, latest);
                let merkle_roots = self
                    .eth_api
                    .fetch_merkle_roots_in_range(current_block, latest)
                    .await?;

                if !merkle_roots.is_empty() {
                    log::info!("Found {} merkle roots", merkle_roots.len());
                }

                for merkle_root in merkle_roots {
                    let block_hash = self
                        .gear_api
                        .block_number_to_hash(merkle_root.block_number as u32)
                        .await?;

                    let authority_set_id =
                        self.gear_api.signed_by_authority_set_id(block_hash).await?;

                    log::info!(
                        "Found merkle root for gear block #{} and era #{}",
                        merkle_root.block_number,
                        authority_set_id
                    );

                    sender.send((
                        authority_set_id,
                        RelayedMerkleRoot {
                            gear_block: merkle_root.block_number as u32,
                        },
                    ))?;
                }

                current_block = latest + 1;
            } else {
                thread::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2)
            }
        }
    }
}

struct Era {
    latest_merkle_root: Option<RelayedMerkleRoot>,
    messages: BTreeMap<BlockNumber, Vec<Message>>,
    pending_txs: Vec<RelayMessagePendingTx>,
}

struct RelayMessagePendingTx {
    hash: TxHash,
    message_block: u32,
    message: Message,
}

struct MessageProcessor {
    eth_api: EthApi,
    gear_api: GearApi,
    block_events: Receiver<BlockEvent>,
    merkle_roots: Receiver<(AuthoritySetId, RelayedMerkleRoot)>,
}

impl MessageProcessor {
    fn new(
        eth_api: EthApi,
        gear_api: GearApi,
        block_events: Receiver<BlockEvent>,
        merkle_roots: Receiver<(AuthoritySetId, RelayedMerkleRoot)>,
    ) -> Self {
        Self {
            eth_api,
            gear_api,
            block_events,
            merkle_roots,
        }
    }

    async fn run(self) {
        loop {
            let res = self.run_inner().await;
            if let Err(err) = res {
                log::error!("Message relayer failed: {}", err);
            }
        }
    }

    async fn run_inner(&self) -> anyhow::Result<()> {
        let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

        let mut paid_messages = HashSet::new();

        loop {
            for event in self.block_events.try_iter() {
                match event {
                    BlockEvent::MessageSent { message } => {
                        let authority_set_id = self
                            .gear_api
                            .signed_by_authority_set_id(message.block_hash)
                            .await?;

                        match eras.entry(authority_set_id) {
                            Entry::Occupied(mut entry) => {
                                match entry.get_mut().messages.entry(message.block) {
                                    Entry::Occupied(mut entry) => {
                                        entry.get_mut().push(message.message);
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(vec![message.message]);
                                    }
                                }
                            }
                            Entry::Vacant(entry) => {
                                let mut messages = BTreeMap::new();
                                messages.insert(message.block, vec![message.message]);

                                entry.insert(Era {
                                    latest_merkle_root: None,
                                    messages,
                                    pending_txs: vec![],
                                });
                            }
                        }
                    }
                    BlockEvent::MessagePaid { nonce } => {
                        paid_messages.insert(nonce);
                    }
                }
            }

            for (authority_set_id, new_merkle_root) in self.merkle_roots.try_iter() {
                match eras.entry(authority_set_id) {
                    Entry::Occupied(mut entry) => {
                        let era = entry.get_mut();

                        if let Some(mr) = era.latest_merkle_root.as_ref() {
                            if mr.gear_block < new_merkle_root.gear_block {
                                era.latest_merkle_root = Some(new_merkle_root);
                            }
                        } else {
                            era.latest_merkle_root = Some(new_merkle_root);
                        }
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(Era {
                            latest_merkle_root: Some(new_merkle_root),
                            messages: BTreeMap::new(),
                            pending_txs: vec![],
                        });
                    }
                }
            }

            let latest_era = eras.last_key_value().map(|(k, _)| *k);
            let Some(latest_era) = latest_era else {
                continue;
            };

            let mut finalized_eras = vec![];

            for (&era_id, era) in eras.iter_mut() {
                let res = era.process(&self.gear_api, &self.eth_api).await;
                if let Err(err) = res {
                    log::error!("Failed to process era #{}: {}", era_id, err);
                    continue;
                }

                let finalized = era.try_finalize(&self.eth_api, &self.gear_api).await?;

                // Latest era cannot be finalized.
                if finalized && era_id != latest_era {
                    log::info!("Era #{} finalized", era_id);
                    finalized_eras.push(era_id);
                }
            }

            for finalized in finalized_eras {
                eras.remove(&finalized);
            }
        }
    }
}

impl Era {
    pub async fn process(&mut self, gear_api: &GearApi, eth_api: &EthApi) -> anyhow::Result<()> {
        let Some(latest_merkle_root) = self.latest_merkle_root else {
            return Ok(());
        };

        let mut processed_blocks = vec![];

        for (&message_block, messages) in self.messages.iter() {
            if message_block > latest_merkle_root.gear_block {
                break;
            }

            let merkle_root_block_hash = gear_api
                .block_number_to_hash(latest_merkle_root.gear_block)
                .await?;

            for message in messages {
                let tx_hash = submit_message(
                    gear_api,
                    eth_api,
                    message,
                    latest_merkle_root.gear_block,
                    merkle_root_block_hash,
                )
                .await?;

                self.pending_txs.push(RelayMessagePendingTx {
                    hash: tx_hash,
                    message_block,
                    message: message.clone(),
                });
            }

            processed_blocks.push(message_block);
        }

        for block in processed_blocks {
            self.messages.remove_entry(&block);
        }

        Ok(())
    }

    pub async fn try_finalize(
        &mut self,
        eth_api: &EthApi,
        gear_api: &GearApi,
    ) -> anyhow::Result<bool> {
        for i in (0..self.pending_txs.len()).rev() {
            self.try_finalize_tx(i, eth_api, gear_api).await?;
        }

        Ok(self.pending_txs.is_empty())
    }

    async fn try_finalize_tx(
        &mut self,
        tx: usize,
        eth_api: &EthApi,
        gear_api: &GearApi,
    ) -> anyhow::Result<bool> {
        let tx = &mut self.pending_txs[tx];
        let status = eth_api.get_tx_status(tx.hash).await?;

        // TODO: Fully decode
        let nonce_bytes: &_ = &tx.message.nonce_le[..16];
        let nonce = u128::from_le_bytes(nonce_bytes.try_into()?);

        match status {
            TxStatus::Finalized => {
                log::info!(
                    "Message at block #{} with nonce {} finalized",
                    tx.message_block,
                    nonce
                );
                Ok(true)
            }
            TxStatus::Pending => {
                log::info!(
                    "Tx for message at block #{} with nonce {} is waiting for finalization",
                    tx.message_block,
                    nonce
                );
                Ok(false)
            }
            TxStatus::Failed => {
                let already_processed = eth_api.is_message_processed(tx.message.nonce_le).await?;

                if already_processed {
                    return Ok(true);
                }

                let merkle_root_block = self
                    .latest_merkle_root
                    .ok_or(anyhow::anyhow!(
                        "Cannot finalize era without any merkle roots"
                    ))?
                    .gear_block;

                if merkle_root_block < tx.message_block {
                    anyhow::bail!(
                        "Cannot relay message at block #{}: latest merkle root is at block #{}",
                        tx.message_block,
                        merkle_root_block
                    );
                }

                let merkle_root_block_hash =
                    gear_api.block_number_to_hash(merkle_root_block).await?;

                let tx_hash = submit_message(
                    gear_api,
                    eth_api,
                    &tx.message,
                    merkle_root_block,
                    merkle_root_block_hash,
                )
                .await?;

                log::warn!(
                    "Retrying to send failed tx {} for message #{}. New tx: {}",
                    hex::encode(tx.hash.0),
                    nonce,
                    hex::encode(tx_hash.0)
                );

                tx.hash = tx_hash;

                Ok(false)
            }
        }
    }
}

async fn submit_message(
    gear_api: &GearApi,
    eth_api: &EthApi,
    message: &Message,
    merkle_root_block: u32,
    merkle_root_block_hash: H256,
) -> anyhow::Result<TxHash> {
    let message_hash = message_hash(message);

    log::info!("Relaying message with hash {}", hex::encode(message_hash));

    let proof = gear_api
        .fetch_message_inclusion_merkle_proof(merkle_root_block_hash, message_hash.into())
        .await?;

    // TODO: Fully decode
    let nonce_bytes = &message.nonce_le[..16];
    let nonce = u128::from_le_bytes(nonce_bytes.try_into()?);

    let tx_hash = eth_api
        .provide_content_message(
            merkle_root_block,
            proof.num_leaves as u32,
            proof.leaf_index as u32,
            nonce,
            message.source,
            message.destination,
            &message.payload[..],
            proof.proof,
        )
        .await?;

    log::info!("Message #{} relaying started", nonce);

    Ok(tx_hash)
}

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}
