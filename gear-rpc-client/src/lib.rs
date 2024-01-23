#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use gsdk::{
    metadata::{
        storage::{BabeStorage, GrandpaStorage, SessionStorage},
        vara_runtime::SessionKeys,
    },
    GearConfig,
};
use parity_scale_codec::{Decode, Encode};
use prover::merkle_proof::{MerkleProof, TrieNodeData};
use sc_consensus_grandpa::{FinalityProof, Precommit};
use sp_consensus_grandpa::GrandpaJustification;
use sp_core::crypto::Wraps;
use sp_runtime::AccountId32;
use subxt::{
    blocks::Block as BlockImpl,
    dynamic::DecodedValueThunk,
    rpc_params,
    storage::{address::Yes, StorageAddress},
    utils::H256,
    OnlineClient,
};
use trie_db::{node::NodeHandle, ChildReference};

const SESSION_KEYS_DATA_LENGTH_IN_BITS: usize = 8 + VALIDATOR_COUNT * (5 * 32) * 8;
const VOTE_LENGTH_IN_BITS: usize = 424;
const VALIDATOR_COUNT: usize = 4;
const PROCESSED_VALIDATOR_COUNT: usize = 3;
const CURRENT_TEST_MESSAGE_LEN_IN_BITS: usize = (1 + VALIDATOR_COUNT * 40) * 8;

const EXPECTED_SESSION_DURATION_IN_BLOCKS: u32 = 1_000;

type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

pub struct GearApi {
    api: gsdk::Api,
}

impl GearApi {
    pub async fn new(endpoint: &str) -> GearApi {
        GearApi {
            api: gsdk::Api::new(Some(endpoint)).await.unwrap(),
        }
    }

    pub async fn block_hash_to_number(&self, block: H256) -> u32 {
        self.api.gear_block_number(Some(block)).await.unwrap()
    }

    pub async fn block_number_to_hash(&self, block: u32) -> H256 {
        self.api
            .rpc()
            .chain_get_block_hash(Some(block.into()))
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn latest_finalized_block(&self) -> H256 {
        self.api.rpc().chain_get_finalized_head().await.unwrap()
    }

    pub async fn validator_set_id(&self, block: H256) -> u64 {
        let block = (*self.api).blocks().at(block).await.unwrap();
        let set_id_address = gsdk::Api::storage_root(GrandpaStorage::CurrentSetId);
        Self::fetch_from_storage(&block, &set_id_address).await
    }

    /// Returns finality proof for block not earlier `after_block`
    /// and not later the end of session this block belongs to.
    pub async fn fetch_finality_proof(
        &self,
        after_block: H256,
    ) -> prover::block_finality::BlockFinality {
        let required_validator_set_id = self.validator_set_id(after_block).await;

        let after_block_number = self.block_hash_to_number(after_block).await;
        let finality: Option<String> = self
            .api
            .rpc()
            .request("grandpa_proveFinality", rpc_params![after_block_number])
            .await
            .unwrap();
        let finality = hex::decode(&finality.unwrap_or_default()["0x".len()..]).unwrap();
        let finality = FinalityProof::<GearHeader>::decode(&mut &finality[..]).unwrap();

        let fetched_validator_set_id = self.validator_set_id(finality.block).await;
        let fetched_block_number = self.block_hash_to_number(finality.block).await;
        assert_eq!(required_validator_set_id, fetched_validator_set_id);
        assert!(fetched_block_number >= after_block_number);

        let justification = finality.justification;
        let justification =
            GrandpaJustification::<GearHeader>::decode(&mut &justification[..]).unwrap();

        for pc in &justification.commit.precommits {
            assert_eq!(pc.precommit.target_hash, finality.block);
            assert_eq!(pc.precommit.target_number, fetched_block_number);
        }

        let signed_data = sp_consensus_grandpa::localized_payload(
            justification.round,
            required_validator_set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(Precommit::<GearHeader>::new(
                finality.block,
                fetched_block_number,
            )),
        );
        assert_eq!(signed_data.len() * 8, VOTE_LENGTH_IN_BITS);

        let validator_set = self.fetch_validator_set(required_validator_set_id).await;
        assert_eq!(VALIDATOR_COUNT, validator_set.len());
        let validator_set = validator_set.try_into().unwrap();

        prover::block_finality::BlockFinality {
            validator_set,
            message: signed_data.try_into().unwrap(),
            pre_commits: justification
                .commit
                .precommits
                .into_iter()
                .take(PROCESSED_VALIDATOR_COUNT)
                .map(|pc| prover::block_finality::PreCommit {
                    public_key: pc.id.as_inner_ref().as_array_ref().to_owned(),
                    signature: pc.signature.as_inner_ref().0.to_owned(),
                })
                .collect(),
        }
    }

    async fn fetch_validator_set(&self, validator_set_id: u64) -> Vec<[u8; 32]> {
        let latest_block = self.latest_finalized_block().await;
        let latest_vs_id = self.validator_set_id(latest_block).await;

        if latest_vs_id == validator_set_id {
            return self.fetch_validator_set_in_block(latest_block).await;
        }

        #[derive(Clone, Copy)]
        enum State {
            SearchBack { latest_bn: u32, step: u32 },
            BinarySearch { lower_bn: u32, higher_bn: u32 },
        }

        let mut state = State::SearchBack {
            latest_bn: self.block_hash_to_number(latest_block).await,
            step: EXPECTED_SESSION_DURATION_IN_BLOCKS,
        };

        loop {
            state = match state {
                State::SearchBack { latest_bn, step } => {
                    let next_bn = latest_bn.saturating_sub(step);
                    let next_block = self.block_number_to_hash(next_bn).await;
                    let next_vs = self.validator_set_id(next_block).await;

                    if next_vs == validator_set_id {
                        return self.fetch_validator_set_in_block(next_block).await;
                    }

                    if next_vs > validator_set_id {
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
                    let mid_block = self.block_number_to_hash(mid_bn).await;
                    let mid_vs = self.validator_set_id(mid_block).await;

                    if mid_vs == validator_set_id {
                        return self.fetch_validator_set_in_block(mid_block).await;
                    }

                    if mid_vs > validator_set_id {
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

    async fn fetch_validator_set_in_block(&self, block: H256) -> Vec<[u8; 32]> {
        let block = (*self.api).blocks().at(block).await.unwrap();

        let session_keys_address = gsdk::Api::storage_root(SessionStorage::QueuedKeys);
        let session_keys: Vec<(AccountId32, SessionKeys)> =
            Self::fetch_from_storage(&block, &session_keys_address).await;

        session_keys
            .into_iter()
            .map(|sc| sc.1.grandpa.0 .0)
            .collect::<Vec<_>>()
    }

    /// NOTE: mock for now, returns some data with constant position in merkle trie.
    pub async fn fetch_sent_message_merkle_proof(
        &self,
        block: H256,
    ) -> MerkleProof<CURRENT_TEST_MESSAGE_LEN_IN_BITS> {
        let address = gsdk::Api::storage_root(BabeStorage::NextAuthorities).to_root_bytes();
        self.fetch_merkle_proof_including_block_header(block, &address)
            .await
    }

    pub async fn fetch_next_session_keys_merkle_proof(
        &self,
        block: H256,
    ) -> MerkleProof<SESSION_KEYS_DATA_LENGTH_IN_BITS> {
        let address = gsdk::Api::storage_root(SessionStorage::QueuedKeys).to_root_bytes();
        self.fetch_merkle_proof_including_block_header(block, &address)
            .await
    }

    async fn fetch_merkle_proof_including_block_header<const LEAF_DATA_LEN_IN_BITS: usize>(
        &self,
        block: H256,
        address: &[u8],
    ) -> MerkleProof<LEAF_DATA_LEN_IN_BITS>
    where
        [(); LEAF_DATA_LEN_IN_BITS / 8]:,
    {
        let merkle_proof = self.fetch_merkle_proof(block, address).await;

        let block = (*self.api).blocks().at(block).await.unwrap();
        let encoded_header = block.header().encode();

        // Assume that encoded_header have the folowing structure:
        // - previous block hash    (32 bytes)
        // - block number           (4 bytes)
        // - merkle state root      (32 bytes)
        // - ...

        let (left_data, rem) = encoded_header.split_at(32 + 4);
        let (state_root_hash, right_data) = rem.split_at(32);

        assert_eq!(state_root_hash, merkle_proof.root_hash);

        let mut merkle_proof_nodes = merkle_proof.nodes;
        merkle_proof_nodes.push(TrieNodeData {
            left_data: left_data.to_vec(),
            right_data: right_data.to_vec(),
        });

        assert_eq!(merkle_proof.leaf_data.len() * 8, LEAF_DATA_LEN_IN_BITS);

        MerkleProof {
            leaf_data: merkle_proof.leaf_data,
            root_hash: block.hash().as_bytes().try_into().unwrap(),
            nodes: merkle_proof_nodes,
        }
    }

    async fn fetch_merkle_proof<const LEAF_DATA_LEN_IN_BITS: usize>(
        &self,
        block: H256,
        address: &[u8],
    ) -> MerkleProof<LEAF_DATA_LEN_IN_BITS>
    where
        [(); LEAF_DATA_LEN_IN_BITS / 8]:,
    {
        use trie_db::{
            node::{Node, Value},
            NodeCodec, TrieLayout,
        };
        type TrieCodec = <sp_trie::LayoutV1<sp_core::Blake2Hasher> as TrieLayout>::Codec;

        let block = (*self.api).blocks().at(block).await.unwrap();

        let storage_keys = vec![address];

        let storage_proof = self
            .api
            .rpc()
            .state_get_read_proof(storage_keys.clone(), Some(block.hash()))
            .await
            .unwrap()
            .proof
            .into_iter()
            .map(|bytes| bytes.0);
        let storage_proof =
            sp_trie::StorageProof::new(storage_proof).to_memory_db::<sp_core::Blake2Hasher>();

        let state_root = block.header().state_root;

        let storage_data = block.storage().fetch_raw(address).await.unwrap().unwrap();

        let mut proof = sp_trie::generate_trie_proof::<
            sp_trie::LayoutV1<sp_core::Blake2Hasher>,
            _,
            _,
            _,
        >(&storage_proof, state_root, storage_keys.iter())
        .unwrap();

        // Note: The following code depends on `TrieCodec` implementation.

        let mut nodes = Vec::with_capacity(proof.len());

        let leaf = proof.pop().unwrap();
        let leaf = TrieCodec::decode(&leaf).unwrap();

        let leaf_node_data = if let Node::Leaf(nibbles, value) = leaf {
            assert!(matches!(value.clone(), Value::Inline(b) if b.is_empty()));
            let mut leaf_data =
                TrieCodec::leaf_node(nibbles.right_iter(), nibbles.len(), Value::Node(&[0; 32]));
            assert_eq!(leaf_data[leaf_data.len() - 32..], [0; 32]);

            leaf_data.resize(leaf_data.len() - 32, 0);

            TrieNodeData {
                left_data: leaf_data,
                right_data: vec![],
            }
        } else {
            panic!("The last node in proof is expected to be leaf");
        };

        nodes.push(leaf_node_data);

        for node_data in proof.iter().rev() {
            let node = TrieCodec::decode(node_data).unwrap();
            let branch_node_data = if let Node::NibbledBranch(nibbles, children, value) = node {
                // There will be only one NodeHandle::Inline(&[]) children and this
                // children will lead to the target leaf.

                let mut target_child_idx = None;
                let children: Vec<Option<ChildReference<H256>>> = children
                    .into_iter()
                    .enumerate()
                    .map(|(child_idx, mut child)| {
                        if matches!(child, Some(NodeHandle::Inline(&[]))) {
                            assert!(target_child_idx.is_none());
                            target_child_idx = Some(child_idx);

                            child = Some(NodeHandle::Hash(&[0; 32]));
                        }

                        child.map(|child| child.try_into().unwrap())
                    })
                    .collect();

                let target_child_idx = target_child_idx.unwrap();

                let mut target_child_offset_from_end = 0;

                #[allow(clippy::needless_range_loop)]
                for child_idx in target_child_idx..children.len() {
                    let serialized_size = match children[child_idx] {
                        Some(ChildReference::Hash(hash)) => hash.as_bytes().encode().len(),
                        Some(ChildReference::Inline(data, len)) => data[..len].encode().len(),
                        None => 0,
                    };
                    target_child_offset_from_end += serialized_size;
                }

                let node_data = TrieCodec::branch_node_nibbled(
                    nibbles.right_iter(),
                    nibbles.len(),
                    children.into_iter(),
                    value,
                );

                // ChildReference::Hash(&[0; 32]) represented as {hash_bytes_length, [hash_bytes]}.
                let (left_data, right_data) = node_data.split_at(
                    node_data.len() - target_child_offset_from_end + 1, /*for length*/
                );

                assert_eq!(right_data[..32], [0; 32]);

                let right_data = &right_data[32..];

                TrieNodeData {
                    left_data: left_data.to_vec(),
                    right_data: right_data.to_vec(),
                }
            } else {
                panic!("All remaining nodes are expected to be nibbled branches");
            };

            nodes.push(branch_node_data);
        }

        assert_eq!(storage_data.len() * 8, LEAF_DATA_LEN_IN_BITS);

        MerkleProof {
            nodes,
            leaf_data: storage_data.try_into().unwrap(),
            root_hash: state_root.0,
        }
    }

    async fn fetch_from_storage<T, A>(
        block: &BlockImpl<GearConfig, OnlineClient<GearConfig>>,
        address: &A,
    ) -> T
    where
        A: StorageAddress<IsFetchable = Yes, Target = DecodedValueThunk>,
        T: Decode,
    {
        let data = block
            .storage()
            .fetch(address)
            .await
            .unwrap()
            .unwrap()
            .into_encoded();

        T::decode(&mut &data[..]).unwrap()
    }
}
