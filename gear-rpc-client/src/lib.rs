#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use std::time::Duration;

use anyhow::{anyhow, Context, Result as AnyResult};
use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use dto::{AuthoritySetState, BranchNodeData};
use futures_util::{Stream, StreamExt};
use gsdk::{
    metadata::{
        gear::Event as GearEvent,
        gear_eth_bridge::Event as GearBridgeEvent,
        runtime_types::{gear_core::message::user::UserMessage, gprimitives::ActorId},
        storage::{GearEthBridgeStorage, GrandpaStorage, SessionStorage, TimestampStorage},
        vara_runtime::SessionKeys,
    },
    Event as RuntimeEvent, GearConfig,
};
use parity_scale_codec::{Compact, Decode, Encode};
use sc_consensus_grandpa::{FinalityProof, Precommit};
use sp_consensus_grandpa::GrandpaJustification;
use sp_core::{crypto::Wraps, Blake2Hasher, Hasher};
use sp_runtime::{traits::AppVerify, AccountId32};
use subxt::{
    backend::BlockRef,
    blocks::Block as BlockImpl,
    dynamic::DecodedValueThunk,
    rpc_params,
    storage::Address,
    utils::{Yes, H256},
    OnlineClient,
};
use trie_db::{node::NodeHandle, ChildReference};

use crate::dto::StorageInclusionProof;

pub mod dto;

pub type GearBlock = BlockImpl<GearConfig, OnlineClient<GearConfig>>;

struct StorageTrieInclusionProof {
    branch_nodes_data: Vec<BranchNodeData>,
    leaf_node_data: Vec<u8>,

    leaf_data: Vec<u8>,
}

const VOTE_LENGTH_IN_BITS: usize = 424;
const APPROX_SESSION_DURATION_IN_BLOCKS: u32 = 1_000;

const MERKLE_ROOT_STORAGE_ADDRESS: &str =
    "fd6e027f7a1bd8baa6406cea4d80d932df509310bc655bbf75a5b563fc3c8eee";
const NEXT_VALIDATOR_SET_ADDRESS: &str =
    "fd6e027f7a1bd8baa6406cea4d80d9327120fd2add6d1249bf1b6bfc3bdf510f";

type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

#[derive(Clone)]
pub struct GearApi {
    pub api: gsdk::Api,
}

impl From<gsdk::Api> for GearApi {
    fn from(api: gsdk::Api) -> Self {
        GearApi { api }
    }
}

impl GearApi {
    pub async fn new(domain: &str, port: u16, retries: u8) -> AnyResult<GearApi> {
        let uri: &str = &format!("{domain}:{port}");
        Ok(GearApi {
            api: gsdk::Api::builder().retries(retries).build(uri).await?,
        })
    }

    /// Get block with the specified hash.
    pub async fn get_block_at(&self, block_hash: H256) -> AnyResult<GearBlock> {
        Ok(self
            .api
            .blocks()
            .at(BlockRef::from_hash(block_hash))
            .await?)
    }

    pub async fn block_hash_to_number(&self, block: H256) -> AnyResult<u32> {
        self.api
            .rpc()
            .chain_get_block(Some(block))
            .await?
            .map(|block| block.block.header.number)
            .ok_or_else(|| anyhow!("Block {block} not present on RPC node"))
    }

    pub async fn block_number_to_hash(&self, block: u32) -> AnyResult<H256> {
        self.api
            .rpc()
            .chain_get_block_hash(Some(block.into()))
            .await?
            .ok_or_else(|| anyhow!("Block #{block} not present on RPC node"))
    }

    pub async fn latest_finalized_block(&self) -> AnyResult<H256> {
        Ok(self.api.rpc().chain_get_finalized_head().await?)
    }

    /// Fetch authority set id for the given block.
    pub async fn authority_set_id(&self, block: H256) -> AnyResult<u64> {
        let block = (*self.api).blocks().at(block).await?;
        let set_id_address = gsdk::Api::storage_root(GrandpaStorage::CurrentSetId);
        Self::fetch_from_storage(&block, &set_id_address).await
    }

    /// Get authority set state for specified block. If block is not specified
    /// the latest finalized block is taken.
    pub async fn authority_set_state(&self, block: Option<H256>) -> AnyResult<AuthoritySetState> {
        let block = match block {
            Some(block) => block,
            None => self.latest_finalized_block().await?,
        };

        let block = (*self.api).blocks().at(block).await?;
        let set_id_address = gsdk::Api::storage_root(GrandpaStorage::CurrentSetId);
        let set_id = Self::fetch_from_storage(&block, &set_id_address).await?;

        let authority_set = self.fetch_authority_set(set_id).await?;
        let authority_set_data: Vec<_> = authority_set.into_iter().flatten().collect();

        let mut hasher = Blake2bVar::new(32).expect("Failed to instantiate Blake2bVar");
        hasher.update(&authority_set_data);
        let mut hash = [0; 32];
        hasher
            .finalize_variable(&mut hash)
            .expect("Hash is of incorrect size");

        Ok(AuthoritySetState {
            authority_set_hash: hash,
            authority_set_id: set_id,
        })
    }

    /// Find authority set id that have signed given `block`.
    pub async fn signed_by_authority_set_id(&self, block: H256) -> AnyResult<u64> {
        let stored_set_id = self.authority_set_id(block).await?;
        let previous_block = self.previous_block(block).await?;
        let previous_block_stored_set_id = self.authority_set_id(previous_block).await?;

        Ok(if previous_block_stored_set_id != stored_set_id {
            previous_block_stored_set_id
        } else {
            stored_set_id
        })
    }

    pub async fn find_era_first_block(&self, authority_set_id: u64) -> AnyResult<H256> {
        let current_set_block = self
            .search_for_authority_set_block(authority_set_id)
            .await?;
        let mut current_set_block = self.block_hash_to_number(current_set_block).await?;

        let mut prev_set_block =
            current_set_block.saturating_sub(APPROX_SESSION_DURATION_IN_BLOCKS);

        loop {
            let prev_set_block_hash = self.block_number_to_hash(prev_set_block).await?;
            let prev_set_id = self.authority_set_id(prev_set_block_hash).await?;

            if prev_set_id < authority_set_id {
                break;
            }

            prev_set_block = prev_set_block.saturating_sub(APPROX_SESSION_DURATION_IN_BLOCKS);
        }

        loop {
            let mid_block = (current_set_block + prev_set_block) / 2;
            let mid_block_hash = self.block_number_to_hash(mid_block).await?;
            let mid_set_id = self.authority_set_id(mid_block_hash).await?;

            if mid_set_id == authority_set_id {
                current_set_block = mid_block;

                let mid_prev = self.previous_block(mid_block_hash).await?;
                let mid_prev_set_id = self.authority_set_id(mid_prev).await?;

                if mid_prev_set_id + 1 == authority_set_id {
                    return Ok(mid_block_hash);
                }
            } else {
                prev_set_block = mid_block;
            }
        }
    }

    async fn previous_block(&self, block: H256) -> AnyResult<H256> {
        let block = self.api.blocks().at(block).await?;
        Ok(block.header().parent_hash)
    }

    pub async fn fetch_finality_proof_for_session(
        &self,
        authority_set_id: u64,
    ) -> AnyResult<(H256, dto::BlockFinalityProof)> {
        let block = self
            .search_for_authority_set_block(authority_set_id)
            .await?;

        self.fetch_finality_proof(block).await
    }

    /// Subscribes to GRANDPA justifications stream and returns it.
    pub async fn subscribe_grandpa_justifications(
        &self,
    ) -> AnyResult<impl Stream<Item = AnyResult<GrandpaJustification<GearHeader>>>> {
        let stream = self
            .api
            .rpc()
            .client()
            .subscribe(
                "grandpa_subscribeJustifications",
                rpc_params![],
                "grandpa_unsubscribeJustifications",
            )
            .await?;

        let stream = stream.map(|res: Result<String, _>| -> AnyResult<_, _> {
            let hex_string = res?;
            let bytes = hex::decode(&hex_string[2..]).context("failed to decoded hex")?;
            let justification = GrandpaJustification::<GearHeader>::decode(&mut &bytes[..])?;

            for pc in &justification.commit.precommits {
                assert_eq!(pc.precommit.target_hash, justification.commit.target_hash);
                assert_eq!(
                    pc.precommit.target_number,
                    justification.commit.target_number
                );
            }

            Ok(justification)
        });

        Ok(stream)
    }

    pub async fn prove_finality(&self, after_block: u32) -> AnyResult<Option<Vec<u8>>> {
        let Some(finality): Option<String> = self
            .api
            .rpc()
            .request::<Option<String>>("grandpa_proveFinality", rpc_params![after_block])
            .await?
        else {
            return Ok(None);
        };
        let finality = hex::decode(finality.strip_prefix("0x").unwrap_or(&finality))?;

        Ok(Some(finality))
    }

    pub async fn fetch_queue_overflowed_since(&self) -> AnyResult<Option<u32>> {
        let block = self.latest_finalized_block().await?;
        let block = (*self.api).blocks().at(block).await?;
        let queue_reset_since = gsdk::Api::storage_root(GearEthBridgeStorage::QueueOverflowedSince);
        self.maybe_fetch_from_storage(&block, &queue_reset_since)
            .await
    }

    /// Returns finality proof for block not earlier `after_block`
    /// and not later the end of session this block belongs to.
    pub async fn fetch_finality_proof(
        &self,
        after_block: H256,
    ) -> AnyResult<(H256, dto::BlockFinalityProof)> {
        let after_block_number = self.block_hash_to_number(after_block).await?;
        let finality: Option<String> = self
            .api
            .rpc()
            .request("grandpa_proveFinality", rpc_params![after_block_number])
            .await?;
        let finality = hex::decode(&finality.unwrap_or_default()["0x".len()..])?;
        let finality = FinalityProof::<GearHeader>::decode(&mut &finality[..])?;

        let fetched_block_number = self.block_hash_to_number(finality.block).await?;
        assert!(fetched_block_number >= after_block_number);

        let justification = finality.justification;
        let justification = GrandpaJustification::<GearHeader>::decode(&mut &justification[..])?;

        for pc in &justification.commit.precommits {
            assert_eq!(pc.precommit.target_hash, finality.block);
            assert_eq!(pc.precommit.target_number, fetched_block_number);
        }

        self.produce_finality_proof(justification).await
    }

    // Produces block finality proof for the given justification.
    pub async fn produce_finality_proof(
        &self,
        justification: GrandpaJustification<GearHeader>,
    ) -> AnyResult<(H256, dto::BlockFinalityProof)> {
        let block_number = justification.commit.target_number;
        let block_hash = justification.commit.target_hash;

        let required_authority_set_id = self.signed_by_authority_set_id(block_hash).await?;

        let signed_data = sp_consensus_grandpa::localized_payload(
            justification.round,
            required_authority_set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(Precommit::<GearHeader>::new(
                block_hash,
                block_number,
            )),
        );
        assert_eq!(signed_data.len() * 8, VOTE_LENGTH_IN_BITS);

        for pc in &justification.commit.precommits {
            assert!(pc.signature.verify(&signed_data[..], &pc.id));
        }

        let validator_set = self.fetch_authority_set(required_authority_set_id).await?;

        let pre_commits: Vec<_> = justification
            .commit
            .precommits
            .into_iter()
            .map(|pc| dto::PreCommit {
                public_key: pc.id.as_inner_ref().as_array_ref().to_owned(),
                signature: pc.signature.as_inner_ref().0.to_owned(),
            })
            .collect();

        Ok((
            block_hash,
            dto::BlockFinalityProof {
                validator_set,
                message: signed_data,
                pre_commits,
            },
        ))
    }

    async fn fetch_authority_set(&self, authority_set_id: u64) -> AnyResult<Vec<[u8; 32]>> {
        let block = self
            .search_for_authority_set_block(authority_set_id)
            .await?;
        self.fetch_authority_set_in_block(block).await
    }

    pub async fn search_for_authority_set_block(&self, authority_set_id: u64) -> AnyResult<H256> {
        let latest_block = self.latest_finalized_block().await?;
        let latest_vs_id = self.authority_set_id(latest_block).await?;

        if latest_vs_id == authority_set_id {
            return Ok(latest_block);
        }

        #[derive(Clone, Copy)]
        enum State {
            SearchBack { latest_bn: u32, step: u32 },
            BinarySearch { lower_bn: u32, higher_bn: u32 },
        }

        let mut state = State::SearchBack {
            latest_bn: self.block_hash_to_number(latest_block).await?,
            step: APPROX_SESSION_DURATION_IN_BLOCKS,
        };

        loop {
            state = match state {
                State::SearchBack { latest_bn, step } => {
                    let next_bn = latest_bn.saturating_sub(step);
                    let next_block = self.block_number_to_hash(next_bn).await?;
                    let next_vs = self.authority_set_id(next_block).await?;

                    if next_vs == authority_set_id {
                        return Ok(next_block);
                    }

                    if next_vs > authority_set_id {
                        State::SearchBack {
                            latest_bn: next_bn,
                            step: step * 2,
                        }
                    } else {
                        State::BinarySearch {
                            lower_bn: next_bn,
                            higher_bn: latest_bn,
                        }
                    }
                }
                State::BinarySearch {
                    lower_bn,
                    higher_bn,
                } => {
                    let mid_bn = (lower_bn + higher_bn) / 2;
                    let mid_block = self.block_number_to_hash(mid_bn).await?;
                    let mid_vs = self.authority_set_id(mid_block).await?;

                    if mid_vs == authority_set_id {
                        return Ok(mid_block);
                    }

                    if mid_vs > authority_set_id {
                        State::BinarySearch {
                            lower_bn,
                            higher_bn: mid_bn,
                        }
                    } else {
                        State::BinarySearch {
                            lower_bn: mid_bn,
                            higher_bn,
                        }
                    }
                }
            }
        }
    }

    async fn fetch_authority_set_in_block(&self, block: H256) -> AnyResult<Vec<[u8; 32]>> {
        let block = (*self.api).blocks().at(block).await?;

        let session_keys_address = gsdk::Api::storage_root(SessionStorage::QueuedKeys);
        let session_keys: Vec<(AccountId32, SessionKeys)> =
            Self::fetch_from_storage(&block, &session_keys_address).await?;

        Ok(session_keys
            .into_iter()
            .map(|sc| sc.1.grandpa.0)
            .collect::<Vec<_>>())
    }

    pub async fn fetch_sent_message_inclusion_proof(
        &self,
        block: H256,
    ) -> AnyResult<dto::StorageInclusionProof> {
        let address = hex::decode(MERKLE_ROOT_STORAGE_ADDRESS).unwrap();
        self.fetch_block_inclusion_proof(block, &address).await
    }

    pub async fn fetch_next_session_keys_inclusion_proof(
        &self,
        block: H256,
    ) -> AnyResult<dto::StorageInclusionProof> {
        let address = hex::decode(NEXT_VALIDATOR_SET_ADDRESS).unwrap();
        self.fetch_block_inclusion_proof(block, &address).await
    }

    async fn fetch_block_inclusion_proof(
        &self,
        block: H256,
        address: &[u8],
    ) -> AnyResult<dto::StorageInclusionProof> {
        let storage_inclusion_proof = self.fetch_storage_inclusion_proof(block, address).await?;

        let block = (*self.api).blocks().at(block).await?;
        let encoded_header = block.header().encode();

        // Assume that encoded_header have the folowing structure:
        // - previous block hash    (32 bytes)
        // - block number           (4 bytes)
        // - merkle state root      (32 bytes)
        // - ...
        let root_node = storage_inclusion_proof
            .branch_nodes_data
            .last()
            .expect("At least one node in storage inclusion proof");
        let fetched_storage_root_hash = Blake2Hasher::hash(&root_node.data);

        let block_number_length = Compact::<u32>(block.number()).encode().len();
        assert_eq!(
            &encoded_header[32 + block_number_length..32 + block_number_length + 32],
            &fetched_storage_root_hash.0
        );

        Ok(StorageInclusionProof {
            address: address.to_vec(),
            block_header: encoded_header,
            branch_nodes_data: storage_inclusion_proof.branch_nodes_data,
            leaf_node_data: storage_inclusion_proof.leaf_node_data,
            stored_data: storage_inclusion_proof.leaf_data,
        })
    }

    async fn fetch_storage_inclusion_proof(
        &self,
        block: H256,
        address: &[u8],
    ) -> AnyResult<StorageTrieInclusionProof> {
        use trie_db::{
            node::{Node, Value},
            NodeCodec, TrieLayout,
        };
        type TrieCodec = <sp_trie::LayoutV1<sp_core::Blake2Hasher> as TrieLayout>::Codec;

        let block = (*self.api).blocks().at(block).await?;

        let storage_keys = vec![address];

        let storage_proof = self
            .api
            .rpc()
            .state_get_read_proof(storage_keys.clone(), Some(block.hash()))
            .await?
            .proof
            .into_iter()
            .map(|bytes| bytes.0);
        let storage_proof =
            sp_trie::StorageProof::new(storage_proof).to_memory_db::<sp_core::Blake2Hasher>();

        let state_root = block.header().state_root;

        let storage_data = block
            .storage()
            .fetch_raw(address)
            .await?
            .ok_or_else(|| anyhow!("Storage at address {address:?} doesn't exist"))?;

        let mut proof = sp_trie::generate_trie_proof::<
            sp_trie::LayoutV1<sp_core::Blake2Hasher>,
            _,
            _,
            _,
        >(&storage_proof, state_root, storage_keys.iter())
        .unwrap_or_else(|err| panic!("Failed to generate trie proof for {address:?}: {err}"));

        let leaf = proof.pop().expect("At least one node in trie proof");
        let leaf = TrieCodec::decode(&leaf).expect("Failed to decode last node in trie proof");
        let encoded_leaf = if let Node::Leaf(nibbles, value) = leaf {
            assert!(matches!(value.clone(), Value::Inline(b) if b.is_empty()));

            let storage_data_hash = Blake2Hasher::hash(&storage_data).0;

            let value = match storage_data.len() {
                32 => Value::Inline(&storage_data),
                l if l > 32 => Value::Node(&storage_data_hash),
                _ => panic!("Unsupported leaf data length"),
            };

            TrieCodec::leaf_node(nibbles.right_iter(), nibbles.len(), value)
        } else {
            panic!("The last node in proof is expected to be leaf");
        };

        let mut current_hash = Blake2Hasher::hash(&encoded_leaf).0;
        let mut branch_nodes = Vec::with_capacity(proof.len());
        for node_data in proof.iter().rev() {
            let node = TrieCodec::decode(node_data).expect("Correctly encoded node");
            if let Node::NibbledBranch(nibbles, children, value) = node {
                // There will be only one NodeHandle::Inline(&[]) children and this
                // children will lead to the target leaf.
                let mut target_child_nibble = None;
                let children: Vec<Option<ChildReference<H256>>> = children
                    .into_iter()
                    .enumerate()
                    .map(|(child_nibble, mut child)| {
                        if matches!(child, Some(NodeHandle::Inline(&[]))) {
                            assert!(target_child_nibble.is_none());
                            target_child_nibble = Some(child_nibble as u8);
                            child = Some(NodeHandle::Hash(&current_hash));
                        }

                        child.map(|child| {
                            child
                                .try_into()
                                .expect("Failed to convert NodeHandle to ChildReference")
                        })
                    })
                    .collect();

                let target_child_nibble = target_child_nibble
                    .expect("At least one child should be NodeHandle::Inline([])");

                let encoded_node = TrieCodec::branch_node_nibbled(
                    nibbles.right_iter(),
                    nibbles.len(),
                    children.into_iter(),
                    value,
                );

                current_hash = Blake2Hasher::hash(&encoded_node).0;

                branch_nodes.push(BranchNodeData {
                    data: encoded_node,
                    target_child: target_child_nibble,
                });
            } else {
                panic!("All remaining nodes are expected to be nibbled branches");
            };
        }

        assert_eq!(state_root.0, current_hash);

        Ok(StorageTrieInclusionProof {
            branch_nodes_data: branch_nodes,
            leaf_node_data: encoded_leaf,
            leaf_data: storage_data,
        })
    }

    async fn maybe_fetch_from_storage<T, A>(
        &self,
        block: &BlockImpl<GearConfig, OnlineClient<GearConfig>>,
        address: &A,
    ) -> AnyResult<Option<T>>
    where
        A: Address<IsFetchable = Yes, Target = DecodedValueThunk>,
        T: Decode,
    {
        let data = match block.storage().fetch(address).await? {
            Some(data) => data.into_encoded(),
            None => return Ok(None),
        };

        Ok(Some(T::decode(&mut &data[..])?))
    }

    async fn fetch_from_storage<T, A>(
        block: &BlockImpl<GearConfig, OnlineClient<GearConfig>>,
        address: &A,
    ) -> AnyResult<T>
    where
        A: Address<IsFetchable = Yes, Target = DecodedValueThunk>,
        T: Decode,
    {
        let data = block
            .storage()
            .fetch(address)
            .await?
            .ok_or_else(|| anyhow!("Block #{} is not present on RPC node", block.number()))?
            .into_encoded();

        Ok(T::decode(&mut &data[..])?)
    }

    pub async fn fetch_message_inclusion_merkle_proof(
        &self,
        block: H256,
        message_hash: H256,
    ) -> AnyResult<dto::MerkleProof> {
        use pallet_gear_eth_bridge_primitives::Proof;

        let proof: Option<Proof> = self
            .api
            .rpc()
            .request(
                "gearEthBridge_merkleProof",
                rpc_params![message_hash, Some(block)],
            )
            .await?;

        let proof = proof.ok_or_else(|| {
            anyhow!("Message with hash {message_hash} not found in block {block}",)
        })?;

        Ok(dto::MerkleProof {
            root: proof.root.0,
            proof: proof.proof.into_iter().map(|h| h.0).collect(),
            num_leaves: proof.number_of_leaves,
            leaf_index: proof.leaf_index,
        })
    }

    pub async fn fetch_timestamp(&self, block: H256) -> AnyResult<u64> {
        let block = (*self.api).blocks().at(block).await?;
        let timestamp_address = gsdk::Api::storage_root(TimestampStorage::Now);
        Self::fetch_from_storage(&block, &timestamp_address)
            .await
            .map(Duration::from_millis)
            .map(|d| d.as_secs())
    }

    /// Fetch queue merkle root for the given block.
    pub async fn fetch_queue_merkle_root(&self, block: H256) -> AnyResult<(u64, H256)> {
        let block = (*self.api).blocks().at(block).await?;
        let set_id_address = gsdk::Api::storage_root(GearEthBridgeStorage::QueueMerkleRoot);
        let merkle_root: H256 = Self::fetch_from_storage(&block, &set_id_address).await?;
        let set_id_address = gsdk::Api::storage_root(GearEthBridgeStorage::QueueId);
        let queue_id: u64 = self.maybe_fetch_from_storage(&block, &set_id_address).await?.unwrap_or_else(|| {
            log::warn!("QueueId not found in storage, using 0 as default");
            0
        });

        Ok((queue_id, merkle_root))
    }

    pub async fn message_queued_events(&self, block: H256) -> AnyResult<Vec<dto::Message>> {
        let events = self.api.get_events_at(Some(block)).await?;

        let events = events.into_iter().filter_map(|event| {
            if let RuntimeEvent::GearEthBridge(GearBridgeEvent::MessageQueued { message, .. }) =
                event
            {
                let mut nonce_be = [0; 32];
                primitive_types::U256(message.nonce.0).to_big_endian(&mut nonce_be);

                Some(dto::Message {
                    nonce_be,
                    source: message.source.0,
                    destination: message.destination.0,
                    payload: message.payload,
                })
            } else {
                None
            }
        });

        Ok(events.collect())
    }

    pub async fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
        block: H256,
    ) -> AnyResult<Vec<dto::UserMessageSent>> {
        let events = self.api.get_events_at(Some(block)).await?;

        let events = events.into_iter().filter_map(|event| {
            let RuntimeEvent::Gear(GearEvent::UserMessageSent {
                message:
                    UserMessage {
                        source,
                        destination,
                        payload,
                        ..
                    },
                ..
            }) = event
            else {
                return None;
            };

            if source != ActorId(from_program.0) {
                return None;
            }

            if destination != ActorId(to_user.0) {
                return None;
            }

            Some(dto::UserMessageSent { payload: payload.0 })
        });

        Ok(events.collect())
    }

    pub async fn get_constant(
        &self,
        pallete: &str,
        constant: &str,
    ) -> AnyResult<DecodedValueThunk> {
        let addr = subxt::dynamic::constant(pallete, constant);
        let res = self
            .api
            .constants()
            .at(&addr)
            .context(format!("Failed to get {pallete}.{constant} address"))?;
        Ok(res)
    }

    pub async fn bridge_admin(&self) -> AnyResult<AccountId32> {
        self.get_constant("GearEthBridge", "BridgeAdmin")
            .await
            .and_then(|res| {
                AccountId32::decode(&mut res.encoded())
                    .context("Failed to decode BridgeAdmin address")
            })
    }

    pub async fn bridge_pauser(&self) -> AnyResult<AccountId32> {
        self.get_constant("GearEthBridge", "BridgePauser")
            .await
            .and_then(|res| {
                AccountId32::decode(&mut res.encoded())
                    .context("Failed to decode BridgePauser address")
            })
    }
}
