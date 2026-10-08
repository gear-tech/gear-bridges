use crate::message_relayer::{
    common::{EthereumSlotNumber, TxHashWithSlot},
    eth_to_gear::storage::{InboundRuntimeIdentity, Storage},
};
use anyhow::Context;
use ethereum_client::PollingEthApi;
use ethereum_common::SECONDS_PER_SLOT;
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc::UnboundedSender, RwLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockCursor {
    pub number: u64,
    pub hash: H256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryState {
    schema_version: u32,
    identity: InboundRuntimeIdentity,
    pub discovered: Option<BlockCursor>,
    pub extracted: Option<BlockCursor>,
    pub pending: BTreeMap<u64, H256>,
}

impl DiscoveryState {
    pub fn next_block(&self) -> anyhow::Result<u64> {
        match self.discovered {
            Some(cursor) => cursor
                .number
                .checked_add(1)
                .context("HOLD: Ethereum discovery block number overflow"),
            None => Ok(self.identity.ethereum_start_block),
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        let start = self.identity.ethereum_start_block;
        let mut next = match self.extracted {
            Some(cursor) => {
                anyhow::ensure!(
                    cursor.number >= start && cursor.hash != H256::zero(),
                    "HOLD: invalid Ethereum extraction cursor"
                );
                cursor
                    .number
                    .checked_add(1)
                    .context("HOLD: extraction cursor overflow")?
            }
            None => start,
        };
        for (&number, &hash) in &self.pending {
            anyhow::ensure!(
                number == next && hash != H256::zero(),
                "HOLD: Ethereum discovery journal has missing or conflicting coverage"
            );
            next = next
                .checked_add(1)
                .context("HOLD: discovery cursor overflow")?;
        }
        let last = self
            .pending
            .last_key_value()
            .map(|(&number, &hash)| BlockCursor { number, hash })
            .or(self.extracted);
        anyhow::ensure!(
            self.discovered == last,
            "HOLD: Ethereum discovered/extracted cursors disagree"
        );
        Ok(())
    }
}

pub struct JSONBlockStorage {
    path: PathBuf,
    state: RwLock<DiscoveryState>,
}

impl JSONBlockStorage {
    pub async fn new(path: PathBuf, identity: InboundRuntimeIdentity) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !tokio::fs::try_exists(path.with_extension("tmp")).await?,
            "HOLD: incomplete Ethereum discovery journal at {}; preserve it for reconciliation",
            path.display()
        );
        let exists = tokio::fs::try_exists(&path).await?;
        let state = if exists {
            let content = tokio::fs::read(&path).await?;
            let state: DiscoveryState = serde_json::from_slice(&content).with_context(|| {
                format!("HOLD: unbound or malformed Ethereum discovery journal at {}; explicit authenticated reconciliation is required", path.display())
            })?;
            anyhow::ensure!(
                state.schema_version == 1 && state.identity == identity,
                "HOLD: Ethereum discovery journal schema or immutable runtime identity changed"
            );
            state.validate()?;
            state
        } else {
            DiscoveryState {
                schema_version: 1,
                identity,
                discovered: None,
                extracted: None,
                pending: BTreeMap::new(),
            }
        };
        let storage = Self {
            path,
            state: RwLock::new(state),
        };
        if !exists {
            storage.save(&*storage.state.read().await).await?;
        }
        Ok(storage)
    }

    pub async fn snapshot(&self) -> DiscoveryState {
        self.state.read().await.clone()
    }

    async fn save(&self, state: &DiscoveryState) -> anyhow::Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        tokio::fs::create_dir_all(parent).await?;
        let temp_path = self.path.with_extension("tmp");
        anyhow::ensure!(
            !tokio::fs::try_exists(&temp_path).await?,
            "HOLD: interrupted Ethereum discovery snapshot"
        );
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await?;
        file.write_all(&serde_json::to_vec(state)?).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(temp_path, &self.path).await?;
        tokio::fs::File::open(parent).await?.sync_all().await?;
        Ok(())
    }

    pub async fn add_block(&self, block: BlockCursor, parent_hash: H256) -> anyhow::Result<()> {
        let mut state = self.state.write().await;
        anyhow::ensure!(
            block.number == state.next_block()? && block.hash != H256::zero(),
            "HOLD: Ethereum discovery is not contiguous"
        );
        if let Some(previous) = state.discovered {
            anyhow::ensure!(
                parent_hash == previous.hash,
                "HOLD: Ethereum finalized discovery ancestry changed"
            );
        } else if block.number == 0 {
            anyhow::ensure!(
                block.hash == state.identity.ethereum_genesis_hash,
                "HOLD: Ethereum discovery genesis changed"
            );
        }
        let mut updated = state.clone();
        updated.discovered = Some(block);
        updated.pending.insert(block.number, block.hash);
        self.save(&updated).await?;
        *state = updated;
        Ok(())
    }

    pub async fn is_pending(&self, block: BlockCursor) -> anyhow::Result<bool> {
        let state = self.state.read().await;
        if let Some(extracted) = state.extracted {
            if block.number <= extracted.number {
                anyhow::ensure!(
                    block.number != extracted.number || block.hash == extracted.hash,
                    "HOLD: finalized extracted block hash changed"
                );
                return Ok(false);
            }
        }
        anyhow::ensure!(
            state.pending.first_key_value() == Some((&block.number, &block.hash)),
            "HOLD: extracted block is absent, changed or out of order in discovery journal"
        );
        Ok(true)
    }

    /// Call only after the extracted transaction hashes are durably stored for handoff.
    pub async fn acknowledge_block(&self, block: BlockCursor) -> anyhow::Result<()> {
        let mut state = self.state.write().await;
        anyhow::ensure!(
            state.pending.first_key_value() == Some((&block.number, &block.hash)),
            "HOLD: cannot acknowledge missing, conflicting or noncontiguous extraction"
        );
        let mut updated = state.clone();
        updated.extracted = Some(block);
        updated.pending.remove(&block.number);
        self.save(&updated).await?;
        *state = updated;
        Ok(())
    }
}

pub(super) async fn finalized_block(
    api: &PollingEthApi,
    number: u64,
) -> anyhow::Result<alloy::rpc::types::Block> {
    let block = api.get_block(number).await?;
    anyhow::ensure!(
        block.header.number == number && block.header.inner.hash_slow() == block.header.hash,
        "HOLD: Ethereum header number/hash is not locally authentic"
    );
    let finalized = api
        .is_finalized_block(number, block.header.hash.0.into())
        .await
        .map_err(|error| {
            if matches!(
                error.downcast_ref::<ethereum_client::Error>(),
                Some(ethereum_client::Error::FinalizedAncestryPending)
            ) {
                anyhow::Error::new(crate::rpc::RpcFailure {
                    operation: "inbound finalized discovery ancestry",
                    kind: crate::rpc::RpcFailureKind::Recoverable,
                    source: error,
                })
            } else {
                error
            }
        })?;
    anyhow::ensure!(
        finalized,
        "HOLD: Ethereum block is not canonically finalized"
    );
    Ok(block)
}

pub(super) async fn replay_pending_handoffs(
    api: &PollingEthApi,
    storage: &dyn Storage,
    genesis_time: u64,
    sender: &UnboundedSender<TxHashWithSlot>,
) -> anyhow::Result<()> {
    let pending = storage
        .block_storage()
        .blocks_raw()
        .read()
        .await
        .iter()
        .filter(|(_, block)| !block.is_processed())
        .map(|(&slot, block)| (slot, block.number, block.hash, block.transactions.clone()))
        .collect::<Vec<_>>();
    for (slot, number, hash, transactions) in pending {
        let hash =
            hash.context("HOLD: pending inbound handoff lacks its immutable EL block hash")?;
        let block = finalized_block(api, number.0).await?;
        let timestamp = block
            .header
            .timestamp
            .checked_sub(genesis_time)
            .context("HOLD: Ethereum block timestamp precedes Beacon genesis")?;
        anyhow::ensure!(
            block.header.hash.0 == hash.0
                && slot == EthereumSlotNumber(timestamp / SECONDS_PER_SLOT),
            "HOLD: pending inbound handoff historical block/slot changed"
        );
        for tx_hash in transactions {
            sender.send(TxHashWithSlot {
                slot_number: slot,
                tx_hash,
            })?;
        }
    }
    Ok(())
}

pub(super) async fn store_extracted_transactions(
    storage: &dyn Storage,
    discovery: &JSONBlockStorage,
    slot: EthereumSlotNumber,
    block: BlockCursor,
    transactions: impl Iterator<Item = ethereum_client::TxHash>,
) -> anyhow::Result<()> {
    storage
        .block_storage()
        .add_block(
            slot,
            crate::message_relayer::common::EthereumBlockNumber(block.number),
            block.hash,
            transactions,
        )
        .await?;
    storage.save_blocks().await?;
    discovery.acknowledge_block(block).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use primitive_types::H160;

    pub(crate) fn identity() -> InboundRuntimeIdentity {
        InboundRuntimeIdentity {
            ethereum_chain_id: 560048,
            ethereum_genesis_hash: H256::repeat_byte(1),
            ethereum_start_block: 70,
            erc20_manager_address: Some(H160::repeat_byte(2)),
            bridging_payment_address: None,
            gear_genesis_hash: H256::repeat_byte(3),
            vft_manager_address: H256::repeat_byte(4),
            checkpoint_light_client_address: H256::repeat_byte(5),
            historical_proxy_address: H256::repeat_byte(6),
            gear_sender: [7; 32],
        }
    }

    #[tokio::test]
    async fn discovery_acknowledges_only_contiguous_durable_pending_coverage() {
        let path =
            std::env::temp_dir().join(format!("ethereum-discovery-{}.json", uuid::Uuid::now_v7()));
        let storage = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        let mut parent = H256::repeat_byte(9);
        for number in 70..270 {
            let block = BlockCursor {
                number,
                hash: H256::from_low_u64_be(number),
            };
            storage.add_block(block, parent).await.unwrap();
            storage.acknowledge_block(block).await.unwrap();
            parent = block.hash;
        }
        let first = BlockCursor {
            number: 270,
            hash: H256::from_low_u64_be(270),
        };
        storage.add_block(first, parent).await.unwrap();
        let second = BlockCursor {
            number: 271,
            hash: H256::from_low_u64_be(271),
        };
        storage.add_block(second, first.hash).await.unwrap();
        assert!(storage.acknowledge_block(second).await.is_err());
        let restored = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        assert_eq!(
            restored.snapshot().await.pending,
            BTreeMap::from([(270, first.hash), (271, second.hash)])
        );
        assert_eq!(restored.snapshot().await.next_block().unwrap(), 272);
        let temporary = path.with_extension("tmp");
        tokio::fs::create_dir(&temporary).await.unwrap();
        assert!(restored.acknowledge_block(first).await.is_err());
        assert!(restored.is_pending(first).await.unwrap());
        tokio::fs::remove_dir(&temporary).await.unwrap();
        restored.acknowledge_block(first).await.unwrap();
        let restarted = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        assert!(!restarted.is_pending(first).await.unwrap());
        assert!(restarted.is_pending(second).await.unwrap());
        restarted.acknowledge_block(second).await.unwrap();
        assert!(restarted.snapshot().await.pending.is_empty());
        let mut changed = identity();
        changed.ethereum_start_block += 1;
        assert!(JSONBlockStorage::new(path.clone(), changed).await.is_err());
        tokio::fs::write(&path, b"[70,71]").await.unwrap();
        assert!(JSONBlockStorage::new(path.clone(), identity())
            .await
            .is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"[70,71]");
        tokio::fs::remove_file(path).await.unwrap();
    }
    #[tokio::test]
    async fn extraction_cursor_cannot_drop_durable_pending_transaction_handoffs() {
        use crate::message_relayer::eth_to_gear::{
            storage::JSONStorage, tx_manager::TransactionManager,
        };
        use std::sync::Arc;
        let root = std::env::temp_dir().join(format!("ethereum-handoff-{}", uuid::Uuid::now_v7()));
        let path = root.join("discovery.json");
        let transactions_path = root.join("transactions");
        let queue = Arc::new(JSONStorage::new(&transactions_path));
        queue.bind_runtime_identity(identity()).await.unwrap();
        queue
            .save(&TransactionManager::new(queue.clone()))
            .await
            .unwrap();
        let discovery = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        let block = BlockCursor {
            number: 70,
            hash: H256::repeat_byte(9),
        };
        let slot = EthereumSlotNumber(7);
        let transaction = ethereum_client::TxHash::from([8; 32]);
        discovery
            .add_block(block, H256::repeat_byte(10))
            .await
            .unwrap();
        let interrupted_blocks = transactions_path.join("blocks.json.new");
        tokio::fs::create_dir(&interrupted_blocks).await.unwrap();
        assert!(store_extracted_transactions(
            queue.as_ref(),
            &discovery,
            slot,
            block,
            [transaction].into_iter()
        )
        .await
        .is_err());
        assert!(discovery.is_pending(block).await.unwrap());
        tokio::fs::remove_dir(interrupted_blocks).await.unwrap();
        let interrupted_cursor = path.with_extension("tmp");
        tokio::fs::create_dir(&interrupted_cursor).await.unwrap();
        assert!(store_extracted_transactions(
            queue.as_ref(),
            &discovery,
            slot,
            block,
            [transaction].into_iter()
        )
        .await
        .is_err());
        assert!(discovery.is_pending(block).await.unwrap());
        let restored = Arc::new(JSONStorage::new(&transactions_path));
        restored.bind_runtime_identity(identity()).await.unwrap();
        restored
            .load(&TransactionManager::new(restored.clone()))
            .await
            .unwrap();
        assert!(
            restored
                .block_storage()
                .is_transaction_pending(slot, transaction)
                .await
        );
        tokio::fs::remove_dir(interrupted_cursor).await.unwrap();
        store_extracted_transactions(
            restored.as_ref(),
            &discovery,
            slot,
            block,
            [transaction].into_iter(),
        )
        .await
        .unwrap();
        drop(discovery);
        drop(restored);
        let discovery = JSONBlockStorage::new(path.clone(), identity())
            .await
            .unwrap();
        assert_eq!(discovery.snapshot().await.extracted, Some(block));
        assert!(discovery.snapshot().await.pending.is_empty());
        let restored = Arc::new(JSONStorage::new(&transactions_path));
        restored.bind_runtime_identity(identity()).await.unwrap();
        restored
            .load(&TransactionManager::new(restored.clone()))
            .await
            .unwrap();
        assert!(
            restored
                .block_storage()
                .is_transaction_pending(slot, transaction)
                .await
        );
        let blocks = restored.block_storage().blocks_raw().read().await;
        assert_eq!(blocks[&slot].hash, Some(block.hash));
        assert_eq!(
            blocks[&slot].transactions,
            std::collections::HashSet::from([transaction])
        );
        drop(blocks);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
