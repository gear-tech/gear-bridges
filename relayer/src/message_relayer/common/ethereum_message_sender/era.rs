use keccak_hash::keccak_256;
use std::collections::{btree_map::Entry, BTreeMap};

use ethereum_client::{EthApi, TxHash, TxStatus};
use gear_rpc_client::{dto::Message, GearApi};
use primitive_types::H256;
use prometheus::IntCounter;

use utils_prometheus::impl_metered_service;

use crate::message_relayer::common::{GearBlockNumber, MessageInBlock, RelayedMerkleRoot};

pub struct Era {
    latest_merkle_root: Option<RelayedMerkleRoot>,
    messages: BTreeMap<GearBlockNumber, Vec<Message>>,
    pending_txs: Vec<RelayMessagePendingTx>,

    metrics: Metrics,
}

struct RelayMessagePendingTx {
    hash: TxHash,
    message_block: GearBlockNumber,
    message: Message,
}

impl_metered_service! {
    pub struct Metrics {
        total_submitted_txs: IntCounter,
        total_failed_txs: IntCounter,
        total_failed_txs_because_processed: IntCounter,
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            total_submitted_txs: IntCounter::new(
                "ethereum_message_sender_total_submitted_txs",
                "Total amount of txs sent to ethereum",
            )?,
            total_failed_txs: IntCounter::new(
                "ethereum_message_sender_total_failed_txs",
                "Total amount of txs sent to ethereum and failed",
            )?,
            total_failed_txs_because_processed: IntCounter::new(
                "ethereum_message_sender_total_failed_txs_because_processed",
                "Amount of txs sent to ethereum and failed because they've already been processed",
            )?,
        })
    }
}

impl Era {
    pub fn new(metrics: Metrics) -> Self {
        Self {
            latest_merkle_root: None,
            messages: BTreeMap::new(),
            pending_txs: vec![],

            metrics,
        }
    }

    pub fn push_message(&mut self, message: MessageInBlock) {
        match self.messages.entry(message.block) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(message.message);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![message.message]);
            }
        }
    }

    pub fn push_merkle_root(&mut self, merkle_root: RelayedMerkleRoot) {
        if let Some(mr) = self.latest_merkle_root.as_ref() {
            if mr.block < merkle_root.block {
                self.latest_merkle_root = Some(merkle_root);
            }
        } else {
            self.latest_merkle_root = Some(merkle_root);
        }
    }

    pub fn pending_tx_count(&self) -> usize {
        self.pending_txs.len()
    }

    pub async fn process(&mut self, gear_api: &GearApi, eth_api: &EthApi) -> anyhow::Result<()> {
        let Some(latest_merkle_root) = self.latest_merkle_root else {
            return Ok(());
        };

        let mut processed_blocks = vec![];

        for (&message_block, messages) in self.messages.iter() {
            if message_block > latest_merkle_root.block {
                break;
            }

            let merkle_root_block_hash = gear_api
                .block_number_to_hash(latest_merkle_root.block.0)
                .await?;

            for message in messages {
                let tx_hash = submit_message(
                    gear_api,
                    eth_api,
                    message,
                    latest_merkle_root.block,
                    merkle_root_block_hash,
                )
                .await?;

                self.metrics.total_submitted_txs.inc();

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
            if self.try_finalize_tx(i, eth_api, gear_api).await? {
                self.pending_txs.remove(i);
            }
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

        let nonce = H256::from(tx.message.nonce_le);

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
                self.metrics.total_failed_txs.inc();

                let already_processed = eth_api.is_message_processed(tx.message.nonce_le).await?;

                if already_processed {
                    self.metrics.total_failed_txs_because_processed.inc();
                    return Ok(true);
                }

                let merkle_root_block = self
                    .latest_merkle_root
                    .ok_or(anyhow::anyhow!(
                        "Cannot finalize era without any merkle roots"
                    ))?
                    .block;

                if merkle_root_block < tx.message_block {
                    anyhow::bail!(
                        "Cannot relay message at block #{}: latest merkle root is at block #{}",
                        tx.message_block,
                        merkle_root_block
                    );
                }

                let merkle_root_block_hash =
                    gear_api.block_number_to_hash(merkle_root_block.0).await?;

                let tx_hash = submit_message(
                    gear_api,
                    eth_api,
                    &tx.message,
                    merkle_root_block,
                    merkle_root_block_hash,
                )
                .await?;

                self.metrics.total_submitted_txs.inc();

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
    merkle_root_block: GearBlockNumber,
    merkle_root_block_hash: H256,
) -> anyhow::Result<TxHash> {
    let message_hash = message_hash(message);

    log::info!("Relaying message with hash {}", hex::encode(message_hash));

    let proof = gear_api
        .fetch_message_inclusion_merkle_proof(merkle_root_block_hash, message_hash.into())
        .await?;

    let tx_hash = eth_api
        .provide_content_message(
            merkle_root_block.0,
            proof.num_leaves as u32,
            proof.leaf_index as u32,
            message.nonce_le,
            message.source,
            message.destination,
            message.payload.to_vec(),
            proof.proof,
        )
        .await?;

    log::info!("Message #{:?} relaying started", message.nonce_le);

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