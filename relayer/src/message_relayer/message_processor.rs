use std::{
    collections::{btree_map::Entry, BTreeMap, HashSet},
    sync::mpsc::Receiver,
};

use ethereum_client::{Contracts as EthApi, TxHash, TxStatus};
use gear_rpc_client::{dto::Message, GearApi};

use crate::metrics::{impl_metered_service, MeteredService};

use super::{submit_message, AuthoritySetId, BlockEvent, BlockNumber, RelayedMerkleRoot};

pub struct MessageProcessor {
    eth_api: EthApi,
    gear_api: GearApi,

    metrics: Metrics,
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

impl_metered_service! {
    struct Metrics {
        //
    }
}

impl MeteredService for MessageProcessor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
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

impl MessageProcessor {
    pub fn new(eth_api: EthApi, gear_api: GearApi) -> Self {
        Self {
            eth_api,
            gear_api,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        self,
        block_events: Receiver<BlockEvent>,
        merkle_roots: Receiver<RelayedMerkleRoot>,
    ) {
        loop {
            let res = self.run_inner(&block_events, &merkle_roots).await;
            if let Err(err) = res {
                log::error!("Message relayer failed: {}", err);
            }
        }
    }

    async fn run_inner(
        &self,
        block_events: &Receiver<BlockEvent>,
        merkle_roots: &Receiver<RelayedMerkleRoot>,
    ) -> anyhow::Result<()> {
        let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

        let mut paid_messages = HashSet::new();

        loop {
            for event in block_events.try_iter() {
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

            for new_merkle_root in merkle_roots.try_iter() {
                match eras.entry(new_merkle_root.authority_set_id) {
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
