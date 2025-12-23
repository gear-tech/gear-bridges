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
use gsdk::GearConfig;
use metadata::{
    gear::Event as GearEvent,
    gear_eth_bridge::Event as GearBridgeEvent,
    runtime_types::{gear_core::message::user::UserMessage, gprimitives::ActorId},
    Event as RuntimeEvent,
};
use parity_scale_codec::{Compact, Decode, Encode};
use sc_consensus_grandpa::FinalityProof;
use sp_consensus_grandpa::{AuthorityId, GrandpaJustification};
use sp_core::{crypto::Wraps, Blake2Hasher, Hasher};
use sp_runtime::AccountId32;
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

use crate::{
    dto::{RawBlockInclusionProof, StorageInclusionProof},
    metadata::runtime_types::frame_system::EventRecord,
};

pub mod dto;
pub mod metadata;

pub use gsdk::{ext, gp};

pub type GearBlock = BlockImpl<GearConfig, OnlineClient<GearConfig>>;

struct StorageTrieInclusionProof {
    branch_nodes_data: Vec<BranchNodeData>,
    leaf_node_data: Vec<u8>,

    leaf_data: Vec<u8>,
}

const VOTE_LENGTH_IN_BITS: usize = 424;

pub type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

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
        let set_id_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GrandpaStorage::CurrentSetId);
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
        let set_id_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GrandpaStorage::CurrentSetId);
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

    pub async fn find_era_first_block(&self, requested_authority_set_id: u64) -> AnyResult<H256> {
        if requested_authority_set_id == 0 {
            return self.block_number_to_hash(0).await;
        }

        let block_requested_auth_set_id = self
            .search_for_authority_set_block(requested_authority_set_id)
            .await?;
        let block_number_requested_auth_set_id = self
            .block_hash_to_number(block_requested_auth_set_id)
            .await?;

        let approx_session_duration_in_blocks =
            block_number_requested_auth_set_id / requested_authority_set_id as u32;
        let mut block_number_lower =
            block_number_requested_auth_set_id.saturating_sub(approx_session_duration_in_blocks);

        loop {
            let prev_set_block_hash = self.block_number_to_hash(block_number_lower).await?;
            let prev_set_id = self.authority_set_id(prev_set_block_hash).await?;

            if prev_set_id < requested_authority_set_id {
                break;
            }

            block_number_lower =
                block_number_lower.saturating_sub(approx_session_duration_in_blocks);
        }

        let mut block_number_higher = block_number_requested_auth_set_id;
        loop {
            if block_number_lower + 1 == block_number_higher {
                // we don't have to check authority set if of the block since it is
                // guaranteed that auth set if of the lower block is less and hence
                // the higher block number is the result.
                return self.block_number_to_hash(block_number_higher).await;
            }

            let mid_block = (block_number_lower + block_number_higher) / 2;
            let mid_block_hash = self.block_number_to_hash(mid_block).await?;
            let mid_set_id = self.authority_set_id(mid_block_hash).await?;

            if mid_set_id == requested_authority_set_id {
                block_number_higher = mid_block;

                let mid_prev = self.previous_block(mid_block_hash).await?;
                let mid_prev_set_id = self.authority_set_id(mid_prev).await?;

                if mid_prev_set_id + 1 == requested_authority_set_id {
                    return Ok(mid_block_hash);
                }
            } else {
                block_number_lower = mid_block;
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
        let block = self.find_era_first_block(authority_set_id).await?;

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
            let mut justification = GrandpaJustification::<GearHeader>::decode(&mut &bytes[..])?;

            justification.commit.precommits.retain(|pc| {
                if pc.precommit.target_hash == justification.commit.target_hash
                    && pc.precommit.target_number == justification.commit.target_number
                {
                    true
                } else {
                    log::warn!(
                        "Filtered out precommit for potentially future block #{}, hash {:?} from justification stream",
                        pc.precommit.target_number,
                        pc.precommit.target_hash
                    );
                    false
                }
            });

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
        let queue_reset_since = gsdk::Api::storage_root(
            gsdk::metadata::storage::GearEthBridgeStorage::QueueOverflowedSince,
        );
        self.maybe_fetch_from_storage(&block, &queue_reset_since)
            .await
    }

    /// Returns GRANDPA justification for block not earlier `after_block`
    pub async fn get_justification(
        &self,
        after_block: H256,
    ) -> AnyResult<GrandpaJustification<GearHeader>> {
        let after_block_number = self.block_hash_to_number(after_block).await?;
        let (justification, _headers) = self.grandpa_prove_finality(after_block_number).await?;

        Ok(justification)
    }

    pub async fn grandpa_prove_finality(
        &self,
        block_number: <GearHeader as sp_runtime::traits::Header>::Number,
    ) -> AnyResult<(GrandpaJustification<GearHeader>, Vec<GearHeader>)> {
        let proof_finality: Option<String> = self
            .api
            .rpc()
            .request("grandpa_proveFinality", rpc_params![block_number])
            .await?;
        let proof_finality = hex::decode(
            proof_finality
                .as_deref()
                .and_then(|s| s.strip_prefix("0x"))
                .unwrap_or_default(),
        )?;
        let proof_finality = FinalityProof::<GearHeader>::decode(&mut &proof_finality[..])?;
        let fetched_block_number = self.block_hash_to_number(proof_finality.block).await?;

        let mut justification =
            GrandpaJustification::<GearHeader>::decode(&mut &proof_finality.justification[..])?;

        justification.commit.precommits.retain(|pc| {
            if pc.precommit.target_hash == proof_finality.block
                && pc.precommit.target_number == fetched_block_number
            {
                true
            } else {
                log::warn!(
                    "Filtered out precommit for potentially future block #{}, hash {:?} from justification",
                    pc.precommit.target_number,
                    pc.precommit.target_hash
                );
                false
            }
        });

        Ok((justification, proof_finality.unknown_headers))
    }

    /// Returns finality proof for block not earlier `after_block`
    /// and not later the end of session this block belongs to.
    pub async fn fetch_finality_proof(
        &self,
        after_block: H256,
    ) -> AnyResult<(H256, dto::BlockFinalityProof)> {
        let justification = self.get_justification(after_block).await?;

        Ok(self.produce_finality_proof(&justification).await?.into())
    }

    // Produces raw block inclusion proof for the given justification.
    pub async fn produce_finality_proof(
        &self,
        justification: &GrandpaJustification<GearHeader>,
    ) -> AnyResult<RawBlockInclusionProof> {
        self.fetch_raw_block_inclusion_proof(
            justification.commit.target_hash,
            Some(justification.clone()),
        )
        .await
    }

    /// Builds a raw block inclusion proof for the given block hash.
    pub async fn fetch_raw_block_inclusion_proof(
        &self,
        block: H256,
        justification: Option<GrandpaJustification<GearHeader>>,
    ) -> AnyResult<RawBlockInclusionProof> {
        let justification = match justification {
            Some(justification) => justification,
            None => self.get_justification(block).await?,
        };
        let required_authority_set_id = self.signed_by_authority_set_id(block).await?;
        let validator_set = self.fetch_authority_set(required_authority_set_id).await?;

        let pre_commits: Vec<_> = justification
            .commit
            .precommits
            .iter()
            .map(|pc| dto::PreCommit {
                public_key: pc.id.as_inner_ref().as_array_ref().to_owned(),
                signature: pc.signature.as_inner_ref().0.to_owned(),
            })
            .collect();

        Ok(RawBlockInclusionProof {
            justification_round: justification.round,
            required_authority_set_id,
            precommit: (
                justification.commit.target_hash,
                justification.commit.target_number,
            ),
            validator_set,
            block_hash: justification.commit.target_hash,
            block_number: justification.commit.target_number,
            pre_commits,
        })
    }

    pub async fn fetch_authority_set(&self, authority_set_id: u64) -> AnyResult<Vec<[u8; 32]>> {
        let block = self
            .search_for_authority_set_block(authority_set_id)
            .await?;
        self.fetch_authority_set_in_block(block).await
    }

    pub async fn search_for_authority_set_block(
        &self,
        requested_authority_set_id: u64,
    ) -> AnyResult<H256> {
        if requested_authority_set_id == 0 {
            return self.block_number_to_hash(0).await;
        }

        let latest_block = self.latest_finalized_block().await?;
        let latest_as_id = self.authority_set_id(latest_block).await?;
        if latest_as_id == requested_authority_set_id {
            return Ok(latest_block);
        }

        if latest_as_id < requested_authority_set_id {
            return Err(anyhow!("Authority set id of the latest finalized block ({latest_block:?}) is {latest_as_id} but request is to search for {requested_authority_set_id}"));
        }

        #[derive(Clone, Copy)]
        enum State {
            SearchBack { latest_bn: u32, step: u32 },
            BinarySearch { lower_bn: u32, higher_bn: u32 },
        }

        let latest_block_number = self.block_hash_to_number(latest_block).await?;
        let approx_session_duration_in_blocks = latest_block_number / latest_as_id as u32;
        let mut state = State::SearchBack {
            latest_bn: self.block_hash_to_number(latest_block).await?,
            step: approx_session_duration_in_blocks,
        };

        loop {
            state = match state {
                State::SearchBack { latest_bn, step } => {
                    let next_bn = latest_bn.saturating_sub(step);
                    let next_block = self.block_number_to_hash(next_bn).await?;
                    let next_as = self.authority_set_id(next_block).await?;

                    if next_as == requested_authority_set_id {
                        return Ok(next_block);
                    }

                    if next_as > requested_authority_set_id {
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
                    if lower_bn + 1 == higher_bn {
                        let block = self.block_number_to_hash(higher_bn).await?;
                        let auth_set_id = self.authority_set_id(block).await?;
                        if auth_set_id == requested_authority_set_id {
                            return Ok(block);
                        }

                        // we don't have to check `lower_bn` since in that case
                        // (lower_bn + higher_bn) / 2 equals to lower_bn and so will be
                        // checked below.
                    }

                    let mid_bn = (lower_bn + higher_bn) / 2;
                    let mid_block = self.block_number_to_hash(mid_bn).await?;
                    let mid_vs = self.authority_set_id(mid_block).await?;

                    if mid_vs == requested_authority_set_id {
                        return Ok(mid_block);
                    }

                    if mid_vs > requested_authority_set_id {
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

        let address = gsdk::Api::storage_root(gsdk::metadata::storage::GrandpaStorage::Authorities);
        let authorities: Vec<(AuthorityId, u64)> =
            Self::fetch_from_storage(&block, &address).await?;

        Ok(authorities
            .into_iter()
            .map(|(grandpa, _)| <[u8; 32]>::try_from(grandpa.as_ref()))
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub async fn fetch_sent_message_inclusion_proof(
        &self,
        block: H256,
    ) -> AnyResult<dto::StorageInclusionProof> {
        let address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GearEthBridgeStorage::QueueMerkleRoot);

        self.fetch_block_inclusion_proof(block, &address.to_root_bytes())
            .await
    }

    pub async fn fetch_next_session_keys_inclusion_proof(
        &self,
        block: H256,
    ) -> AnyResult<dto::StorageInclusionProof> {
        let address = gsdk::Api::storage_root(
            gsdk::metadata::storage::GearEthBridgeStorage::AuthoritySetHash,
        );
        self.fetch_block_inclusion_proof(block, &address.to_root_bytes())
            .await
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

        if encoded_header[32 + block_number_length..32 + block_number_length + 32]
            != fetched_storage_root_hash.0
        {
            return Err(anyhow!(
                "Storage root hash mismatch: expected {:x?}, got {:x?}",
                &encoded_header[32 + block_number_length..32 + block_number_length + 32],
                &fetched_storage_root_hash.0
            ));
        }

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
            if !matches!(value, Value::Inline(b) if b.is_empty()) {
                return Err(anyhow!(
                    "Expected leaf node to have empty value, got {value:?}",
                ));
            }

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

        if state_root.0 != current_hash {
            return Err(anyhow!(
                "State root hash mismatch: expected {:x?}, got {:x?}",
                state_root.0,
                current_hash
            ));
        }

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
        let timestamp_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::TimestampStorage::Now);
        Self::fetch_from_storage(&block, &timestamp_address)
            .await
            .map(Duration::from_millis)
            .map(|d| d.as_secs())
    }

    /// Fetch queue merkle root for the given block.
    pub async fn fetch_queue_merkle_root(&self, block: H256) -> AnyResult<(u64, H256)> {
        let block = (*self.api).blocks().at(block).await?;
        let set_id_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GearEthBridgeStorage::QueueMerkleRoot);
        let merkle_root: H256 = Self::fetch_from_storage(&block, &set_id_address).await?;
        let set_id_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GearEthBridgeStorage::QueueId);
        let queue_id: u64 = self
            .maybe_fetch_from_storage(&block, &set_id_address)
            .await?
            .unwrap_or_else(|| {
                log::warn!("QueueId not found in storage, using 0 as default");
                0
            });

        Ok((queue_id, merkle_root))
    }

    pub async fn get_events_at(
        &self,
        block_hash: Option<H256>,
    ) -> anyhow::Result<Vec<RuntimeEvent>> {
        let addr = gsdk::Api::storage_root(gsdk::metadata::storage::SystemStorage::Events);

        let evs: Vec<EventRecord<RuntimeEvent, H256>> =
            self.api.fetch_storage_at(&addr, block_hash).await?;
        Ok(evs.into_iter().map(|er| er.event).collect())
    }

    pub async fn message_queued_events(&self, block: H256) -> AnyResult<Vec<dto::Message>> {
        let events = self.get_events_at(Some(block)).await?;

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
        let events = self.get_events_at(Some(block)).await?;

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

#[cfg(test)]
mod tests {

    #[test]
    fn storage_correct() {
        const MERKLE_ROOT_STORAGE_ADDRESS: &str =
            "fd6e027f7a1bd8baa6406cea4d80d932df509310bc655bbf75a5b563fc3c8eee";
        const NEXT_VALIDATOR_SET_ADDRESS: &str =
            "fd6e027f7a1bd8baa6406cea4d80d9327120fd2add6d1249bf1b6bfc3bdf510f";

        let merkle_root_address = hex::decode(MERKLE_ROOT_STORAGE_ADDRESS).unwrap();
        let expected_merkle_root_address =
            gsdk::Api::storage_root(gsdk::metadata::storage::GearEthBridgeStorage::QueueMerkleRoot);
        assert_eq!(
            merkle_root_address,
            expected_merkle_root_address.to_root_bytes()
        );
        let next_validator_set_address = hex::decode(NEXT_VALIDATOR_SET_ADDRESS).unwrap();
        let expected_next_validator_set_address = gsdk::Api::storage_root(
            gsdk::metadata::storage::GearEthBridgeStorage::AuthoritySetHash,
        );
        assert_eq!(
            next_validator_set_address,
            expected_next_validator_set_address.to_root_bytes()
        );
    }
}
