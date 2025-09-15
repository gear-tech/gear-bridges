use crate::{
    merkle_roots::MerkleRoot,
    message_relayer::common::{
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock,
    },
    proof_storage::ProofStorage,
};
use gclient::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use primitive_types::{H256, U256};
use sails_rs::events::EventIo;
use serde::{Deserialize, Serialize};
use std::{
    collections::{btree_map::Entry, BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
};

pub struct MerkleRootStorage {
    pub proofs: Arc<dyn ProofStorage>,
    pub blocks: RwLock<BTreeMap<u32, Block>>,
    pub submitted_roots: RwLock<HashSet<H256>>,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Block {
    pub block_hash: H256,
    pub merkle_root_changed: Option<(u64, H256)>,
    pub authority_set_changed: bool,
}

impl Block {
    pub fn is_processed(&self) -> bool {
        self.merkle_root_changed.is_none() && !self.authority_set_changed
    }
}

pub(super) fn queue_merkle_root_changed(block: &GearBlock) -> Option<(u64, H256)> {
    block.events().iter().find_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::QueueMerkleRootChanged {
            queue_id,
            root,
        }) => Some((*queue_id, *root)),
        _ => None,
    })
}

pub(super) fn message_queued_events_of(block: &GearBlock) -> impl Iterator<Item = U256> + use<'_> {
    block.events().iter().filter_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::MessageQueued { message, .. }) => {
            Some(U256(message.nonce.0))
        }
        _ => None,
    })
}

pub(super) fn priority_bridging_paid<'a>(
    block: &'a GearBlock,
    bridging_payment_address: H256,
) -> impl Iterator<Item = (H256, U256)> + 'a {
    block
        .user_message_sent_events(bridging_payment_address, H256::default())
        .filter_map(|payload| {
            match bridging_payment_client::bridging_payment::events::BridgingPaymentEvents::decode_event(payload) {
                Ok(bridging_payment_client::bridging_payment::events::BridgingPaymentEvents::PriorityBridgingPaid {
                    block,
                    nonce,
                }) => Some((block, nonce)),
                _ => None,
            }
        })
}

pub(super) fn authority_set_changed(block: &GearBlock) -> bool {
    block
        .events()
        .iter()
        .find_map(|event| match event {
            gclient::Event::GearEthBridge(GearEthBridgeEvent::AuthoritySetHashChanged(_))
            | gclient::Event::Grandpa(gclient::metadata::grandpa::Event::NewAuthorities {
                ..
            }) => Some(()),
            _ => None,
        })
        .is_some()
}

#[async_trait::async_trait]
impl UnprocessedBlocksStorage for MerkleRootStorage {
    async fn unprocessed_blocks(&self) -> UnprocessedBlocks {
        let blocks = self.blocks.read().await;
        let first_block = blocks.first_key_value().map(|(k, v)| (v.block_hash, *k));
        UnprocessedBlocks {
            blocks: blocks.iter().map(|(k, v)| (v.block_hash, *k)).collect(),
            last_block: None,
            first_block,
        }
    }

    async fn add_block(&self, block: &GearBlock) {
        let merkle_root_changed = queue_merkle_root_changed(block);
        let authority_set_changed = authority_set_changed(block);

        // in case there are no merkle-root related changes we can just skip the block saving.
        if merkle_root_changed.is_none() && !authority_set_changed {
            return;
        }

        let block_hash = block.hash();
        let block_number = block.number();

        let mut blocks = self.blocks.write().await;
        match blocks.entry(block_number) {
            Entry::Occupied(_) => {
                log::warn!("Block #{block_number} already exists in storage");
                return;
            }

            Entry::Vacant(entry) => {
                entry.insert(Block {
                    block_hash,
                    merkle_root_changed,
                    authority_set_changed,
                });
            }
        }
    }
}

impl MerkleRootStorage {
    pub fn new(proofs: Arc<dyn ProofStorage>, path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            proofs,
            blocks: RwLock::new(BTreeMap::new()),
            submitted_roots: RwLock::new(HashSet::new()),
            path,
        })
    }

    pub async fn is_merkle_root_submitted(&self, merkle_root: H256) -> bool {
        self.submitted_roots.read().await.contains(&merkle_root)
    }

    pub async fn submitted_merkle_root(&self, merkle_root: H256) {
        self.submitted_roots.write().await.insert(merkle_root);
    }

    pub async fn submission_failed(&self, merkle_root: H256) {
        self.submitted_roots.write().await.remove(&merkle_root);
    }

    pub async fn merkle_root_processed(&self, block_number: u32) {
        let mut blocks = self.blocks.write().await;

        let Entry::Occupied(entry) = blocks.entry(block_number).and_modify(|block| {
            block.merkle_root_changed = None;
        }) else {
            return;
        };

        if !entry.get().authority_set_changed {
            entry.remove();
        }
    }

    pub async fn authority_set_processed(&self, block_number: u32) {
        let mut blocks = self.blocks.write().await;

        let Entry::Occupied(entry) = blocks.entry(block_number).and_modify(|block| {
            block.authority_set_changed = false;
        }) else {
            return;
        };

        if entry.get().merkle_root_changed.is_none() {
            entry.remove();
        }
    }

    /// Save unprocessed blocks to the provided path.
    pub async fn save(&self, roots: &HashMap<H256, MerkleRoot>) -> anyhow::Result<()> {
        self.prune_blocks().await;
        let blocks = self.blocks.read().await;
        let submitted_merkle_roots = self.submitted_roots.read().await;

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .await?;

        let storage = SerializedStorage {
            blocks: &blocks,
            submitted_merkle_roots: &submitted_merkle_roots,
            roots,
        };

        let serialized = serde_json::to_string(&storage)?;

        file.write_all(serialized.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    pub async fn load(&self) -> anyhow::Result<HashMap<H256, MerkleRoot>> {
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .open(&self.path)
            .await?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).await?;

        let DeserializedStorage {
            blocks,
            submitted_merkle_roots,
            roots,
        } = serde_json::from_str(&contents)?;
        *self.blocks.write().await = blocks;
        *self.submitted_roots.write().await = submitted_merkle_roots;
        Ok(roots)
    }

    pub async fn prune_blocks(&self) {
        let mut blocks = self.blocks.write().await;
        let mut remove_until = None;
        for (index, (number, block)) in blocks.iter().enumerate() {
            if index + 100 > blocks.len() {
                remove_until = Some(*number);
                break;
            }

            if !block.is_processed() {
                remove_until = Some(*number);
                break;
            }
        }

        if let Some(remove_until) = remove_until {
            *blocks = blocks.split_off(&remove_until);
        }
    }
}

#[derive(Serialize)]
struct SerializedStorage<'a> {
    blocks: &'a BTreeMap<u32, Block>,
    submitted_merkle_roots: &'a HashSet<H256>,
    roots: &'a HashMap<H256, MerkleRoot>,
}

#[derive(Deserialize)]
struct DeserializedStorage {
    blocks: BTreeMap<u32, Block>,
    submitted_merkle_roots: HashSet<H256>,
    roots: HashMap<H256, MerkleRoot>,
}
