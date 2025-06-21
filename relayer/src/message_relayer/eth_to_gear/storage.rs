use super::tx_manager::{Transaction, TransactionManager, TxStatus};
use crate::message_relayer::common::{EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot};
use async_trait::async_trait;
use ethereum_client::{EthApi, TxHash};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
};
use uuid::Uuid;

/// Storage type implementing
/// storage for Ethereum blocks.
pub struct BlockStorage {
    blocks: RwLock<BTreeMap<EthereumSlotNumber, Block>>,
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    pub slot_number: EthereumSlotNumber,
    pub number: EthereumBlockNumber,
    pub transactions: HashSet<TxHash>,
}

impl Block {
    pub fn is_processed(&self) -> bool {
        self.transactions.is_empty()
    }
}

impl BlockStorage {
    pub fn new() -> Self {
        Self {
            blocks: RwLock::new(BTreeMap::new()),
        }
    }

    pub async fn complete_transaction(&self, tx: &TxHashWithSlot) {
        let mut blocks = self.blocks.write().await;
        let Some(block) = blocks.get_mut(&tx.slot_number) else {
            log::warn!(
                "Block at slot #{} associated with transaction #{:?} not found in storage",
                tx.slot_number,
                tx.tx_hash
            );
            return;
        };

        if !block.transactions.remove(&tx.tx_hash) {
            log::warn!(
                "Transaction #{:?} in block at slot #{} is already completed",
                tx.slot_number.0,
                tx.tx_hash
            );
        };
    }

    pub async fn add_block(
        &self,
        slot: EthereumSlotNumber,
        number: EthereumBlockNumber,
        txs: impl Iterator<Item = TxHash>,
    ) {
        if self
            .blocks
            .write()
            .await
            .insert(
                slot,
                Block {
                    slot_number: slot,
                    number,
                    transactions: txs.collect(),
                },
            )
            .is_some()
        {
            log::warn!("Block at slot #{} is already in storage", slot.0);
        };
    }

    pub async fn missing_blocks(
        &self,
        eth_api: &EthApi,
    ) -> anyhow::Result<VecDeque<EthereumBlockNumber>> {
        let blocks = self.blocks.read().await;

        if blocks.is_empty() {
            // if there are no finalized blocks processing will start from latest
            // finalized block
            return Ok(VecDeque::new());
        }

        let mut missing = VecDeque::new();
        let min = blocks
            .iter()
            .min_by_key(|(_, block)| block.number)
            .unwrap()
            .1
            .number;
        let max = blocks
            .iter()
            .max_by_key(|(_, block)| block.number)
            .unwrap()
            .1
            .number;

        let mut expected = min;

        for (_, block) in blocks.iter() {
            while expected < block.number {
                missing.push_back(expected);
                expected.0 += 1;
            }
        }

        while expected <= max {
            missing.push_back(expected);
            expected.0 += 1;
        }

        let latest_finalized = eth_api.finalized_block_number().await?;

        while expected.0 <= latest_finalized {
            missing.push_back(expected);
            expected.0 += 1;
        }

        Ok(missing)
    }

    pub async fn prune(&self, n_to_keep: usize) {
        let mut blocks = self.blocks.write().await;

        let processed_count = blocks
            .iter()
            .filter(|(_, block)| block.is_processed())
            .count();

        if processed_count <= n_to_keep {
            log::debug!("Not pruning block storage. Processed blocks: {processed_count}, Target: {n_to_keep}");
            return;
        }

        let num_to_remove = processed_count - n_to_keep;

        log::debug!("Removing {num_to_remove} processed block(s)");

        let mut removal_candidates = Vec::new();
        let mut current_run = Vec::new();
        let mut last_slot = None::<EthereumSlotNumber>;

        for (&slot, block) in blocks.iter() {
            if block.is_processed() {
                if let Some(last_slot) = last_slot {
                    if slot.0 == last_slot.0 + 1 {
                        current_run.push(slot);
                    } else {
                        // the sequence is broken by a hole, finalize the previous run
                        if !current_run.is_empty() {
                            removal_candidates.append(&mut current_run);
                        }
                        // start a new run
                        current_run.push(slot);
                    }
                } else {
                    current_run.push(slot);
                }
                last_slot = Some(slot);
            } else {
                if !current_run.is_empty() {
                    removal_candidates.append(&mut current_run);
                    current_run.clear();
                }

                last_slot = None;
            }
        }

        if !current_run.is_empty() {
            removal_candidates.append(&mut current_run);
        }

        for slot_to_remove in removal_candidates.iter().take(num_to_remove) {
            log::debug!(
                "Pruning processed block at slot #{} from block storage",
                slot_to_remove.0
            );
            blocks.remove(slot_to_remove);
        }
    }
}

#[async_trait]
pub trait Storage: Send + Sync {
    fn block_storage(&self) -> &BlockStorage;
    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
}

pub struct NoStorage(BlockStorage);

impl NoStorage {
    pub fn new() -> Self {
        Self(BlockStorage::new())
    }
}

#[async_trait]
impl Storage for NoStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.0
    }
    async fn save(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }

    async fn load(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }
}

/// Simple storage for transactions which keeps them in a JSON file under
/// specified directory.
pub struct JSONStorage {
    path: PathBuf,
    block_storage: BlockStorage,
}

impl JSONStorage {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            block_storage: BlockStorage::new(),
        }
    }

    async fn write_tx(&self, tx_uuid: &Uuid, tx: &Transaction) -> anyhow::Result<()> {
        if matches!(tx.status, TxStatus::Completed) {
            self.block_storage().complete_transaction(&tx.tx).await;
        }

        let filename = self.path.join(tx_uuid.to_string());
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(filename)
            .await?;

        let json = serde_json::to_string(&tx)?;
        file.write_all(json.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    async fn read_tx(&self, path: PathBuf, tx_file: OsString) -> anyhow::Result<Transaction> {
        let uuid = tx_file
            .to_str()
            .and_then(|s| Uuid::from_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in filename {tx_file:?}"))?;
        let mut contents = String::new();

        let mut file = tokio::fs::File::open(path).await?;
        file.read_to_string(&mut contents).await?;

        let tx: Transaction = serde_json::from_str(&contents)?;

        if tx.uuid != uuid {
            return Err(anyhow::anyhow!(
                "UUID in filename does not match transaction UUID: {} != {}",
                uuid,
                tx.uuid
            ));
        }

        Ok(tx)
    }
}

#[async_trait]
impl Storage for JSONStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.block_storage
    }

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            tokio::fs::create_dir_all(&self.path).await?;
        }
        let mut failed = BTreeMap::new();

        let failed_map = tx_manager.failed.read().await;
        for (tx_uuid, tx) in tx_manager.transactions.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
            if let Some(reason) = failed_map.get(tx_uuid) {
                failed.insert(*tx_uuid, reason);
            }
        }

        let mut failed_file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.path.join("failed"))
            .await?;

        let str = serde_json::to_string(&failed)?;
        failed_file.write_all(str.as_bytes()).await?;
        failed_file.flush().await?;

        let block_storage_path = self.path.join("blocks.json");

        let mut blocks_file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(block_storage_path)
            .await?;
        // just keep 100 processed blocks in JSON storage for now...
        self.block_storage().prune(100).await;
        let blocks = self.block_storage().blocks.read().await;
        let blocks_json = serde_json::to_string::<BTreeMap<EthereumSlotNumber, Block>>(&*blocks)?;
        blocks_file.write_all(blocks_json.as_bytes()).await?;
        blocks_file.flush().await?;

        Ok(())
    }

    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        let mut dir = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = dir.next_entry().await? {
            if entry.file_type().await?.is_file() {
                if entry.file_name().to_str() == Some("failed") {
                    let contents = tokio::fs::read_to_string(entry.path()).await?;
                    let map: BTreeMap<Uuid, String> = serde_json::from_str(&contents)?;
                    *tx_manager.failed.write().await = map;
                } else if entry.file_name().to_str() == Some("blocks.json") {
                    let contents = tokio::fs::read_to_string(entry.path()).await?;
                    let map: BTreeMap<EthereumSlotNumber, Block> = serde_json::from_str(&contents)?;
                    *self.block_storage().blocks.write().await = map;
                } else {
                    let tx = self.read_tx(entry.path(), entry.file_name()).await?;

                    tx_manager.add_transaction(tx).await;
                }
            }
        }

        Ok(())
    }
}
