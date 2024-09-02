use keccak_hash::keccak_256;
use std::{
    collections::{btree_map::Entry, BTreeMap, HashSet},
    sync::mpsc::Receiver,
};

use ethereum_client::{EthApi, TxHash, TxStatus};
use gear_rpc_client::{dto::Message, GearApi};
use primitive_types::H256;
use prometheus::{Gauge, IntCounter, IntGauge};

use utils_prometheus::{impl_metered_service, MeteredService};

use super::merkle_root_listener::RelayedMerkleRoot;

type BlockNumber = u32;

struct EraTxSubmitter {
    latest_merkle_root: Option<RelayedMerkleRoot>,
    messages: BTreeMap<BlockNumber, Vec<Message>>,
    pending_txs: Vec<PendingTx>,

    eth_api: EthApi,
    gear_api: GearApi,
}

struct PendingTx {
    hash: TxHash,
    message_block: u32,
    message: Message,
}

pub struct MessageInBlock {
    pub message: Message,
    pub block: BlockNumber,
}

impl EraTxSubmitter {
    pub fn new(eth_api: EthApi, gear_api: GearApi) -> Self {
        Self {
            latest_merkle_root: None,
            messages: Default::default(),
            pending_txs: vec![],

            eth_api,
            gear_api,
        }
    }

    pub async fn run(
        mut self,
        messages: Receiver<MessageInBlock>,
        merkle_roots: Receiver<RelayedMerkleRoot>,
    ) {
        loop {
            let res = self.run_inner(&messages, &merkle_roots).await;
            if let Err(err) = res {
                log::error!("Ethereum tx submitter failed: {}", err);
            }
        }
    }

    async fn run_inner(
        &mut self,
        messages: &Receiver<MessageInBlock>,
        merkle_roots: &Receiver<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        loop {
            for merkle_root in merkle_roots.try_iter() {
                if let Some(mr) = self.latest_merkle_root.as_ref() {
                    if mr.gear_block < merkle_root.gear_block {
                        self.latest_merkle_root = Some(merkle_root);
                    }
                } else {
                    self.latest_merkle_root = Some(merkle_root);
                }
            }

            for message in messages.try_iter() {
                match self.messages.entry(message.block) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().push(message.message);
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(vec![message.message]);
                    }
                }
            }

            self.submit_queued_messages().await?;

            for i in (0..self.pending_txs.len()).rev() {
                if self.try_finalize_tx(i).await? {
                    self.pending_txs.remove(i);
                }
            }
        }
    }

    async fn submit_queued_messages(&mut self) -> anyhow::Result<()> {
        let Some(latest_merkle_root) = self.latest_merkle_root else {
            return Ok(());
        };

        let mut processed_blocks = vec![];

        for (&message_block, messages) in self.messages.iter() {
            if message_block > latest_merkle_root.gear_block {
                break;
            }

            let merkle_root_block_hash = self
                .gear_api
                .block_number_to_hash(latest_merkle_root.gear_block)
                .await?;

            for message in messages {
                let tx_hash = self
                    .submit_message(
                        message,
                        latest_merkle_root.gear_block,
                        merkle_root_block_hash,
                    )
                    .await?;

                //self.metrics.total_submitted_txs.inc();

                self.pending_txs.push(PendingTx {
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

    async fn try_finalize_tx(&mut self, tx: usize) -> anyhow::Result<bool> {
        let tx_message = self.pending_txs[tx].message.clone();
        let tx_message_block = self.pending_txs[tx].message_block;
        let tx_hash = self.pending_txs[tx].hash;

        let status = self.eth_api.get_tx_status(tx_hash).await?;

        let nonce = H256::from(tx_message.nonce_le);

        match status {
            TxStatus::Finalized => {
                log::info!(
                    "Message at block #{} with nonce {} finalized",
                    tx_message_block,
                    nonce
                );
                Ok(true)
            }
            TxStatus::Pending => {
                log::info!(
                    "Tx for message at block #{} with nonce {} is waiting for finalization",
                    tx_message_block,
                    nonce
                );
                Ok(false)
            }
            TxStatus::Failed => {
                //self.metrics.total_failed_txs.inc();

                let already_processed = self
                    .eth_api
                    .is_message_processed(tx_message.nonce_le)
                    .await?;

                if already_processed {
                    //self.metrics.total_failed_txs_because_processed.inc();
                    return Ok(true);
                }

                let merkle_root_block = self
                    .latest_merkle_root
                    .ok_or(anyhow::anyhow!(
                        "Cannot finalize era without any merkle roots"
                    ))?
                    .gear_block;

                if merkle_root_block < tx_message_block {
                    anyhow::bail!(
                        "Cannot relay message at block #{}: latest merkle root is at block #{}",
                        tx_message_block,
                        merkle_root_block
                    );
                }

                let merkle_root_block_hash = self
                    .gear_api
                    .block_number_to_hash(merkle_root_block)
                    .await?;

                let new_tx_hash = self
                    .submit_message(&tx_message, merkle_root_block, merkle_root_block_hash)
                    .await?;

                //self.metrics.total_submitted_txs.inc();

                log::warn!(
                    "Retrying to send failed tx {} for message #{}. New tx: {}",
                    hex::encode(tx_hash.0),
                    nonce,
                    hex::encode(new_tx_hash.0)
                );

                self.pending_txs[tx].hash = new_tx_hash;

                Ok(false)
            }
        }
    }

    async fn submit_message(
        &self,
        message: &Message,
        merkle_root_block: u32,
        merkle_root_block_hash: H256,
    ) -> anyhow::Result<TxHash> {
        let message_hash = message_hash(message);

        log::info!("Relaying message with hash {}", hex::encode(message_hash));

        let proof = self
            .gear_api
            .fetch_message_inclusion_merkle_proof(merkle_root_block_hash, message_hash.into())
            .await?;

        let tx_hash = self
            .eth_api
            .provide_content_message(
                merkle_root_block,
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
