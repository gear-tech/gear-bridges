#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use gsdk::metadata::{
    storage::{BabeStorage, GrandpaStorage, SessionStorage},
    vara_runtime::SessionKeys,
};
use parity_scale_codec::{Decode, Encode};
use prover::merkle_proof::{MerkleProof, TrieNodeData};
use sc_consensus_grandpa::FinalityProof;
use sp_core::crypto::Wraps;
use sp_runtime::AccountId32;
use subxt::{rpc_params, utils::H256};
use trie_db::{node::NodeHandle, ChildReference};

const SESSION_KEYS_DATA_LENGTH_IN_BITS: usize = 8 + VALIDATOR_COUNT * (5 * 32) * 8;
const VOTE_LENGTH_IN_BITS: usize = 424;
const VALIDATOR_COUNT: usize = 4;
const PROCESSED_VALIDATOR_COUNT: usize = 3;
const CURRENT_TEST_MESSAGE_LEN_IN_BITS: usize = (1 + VALIDATOR_COUNT * 40) * 8;

type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

pub struct GearApi {
    api: gsdk::Api,
}

impl GearApi {
    pub async fn new() -> GearApi {
        GearApi {
            api: gsdk::Api::new(Some("wss://testnet-archive.vara-network.io:443"))
                .await
                .unwrap(),
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

    // TODO: Process case when `grandpa_proveFinality` yields proof for the other block than specified.
    // Basically this can be in the case when there were finalized blocks between `grandpa_proveFinality`
    // and `latest_finalized_block` calls.
    pub async fn fetch_finality_proof(&self, block: H256) -> prover::block_finality::BlockFinality {
        let block_hash = block;
        let block = (*self.api).blocks().at(block).await.unwrap();

        let finality: Option<String> = self
            .api
            .rpc()
            .request("grandpa_proveFinality", rpc_params![block.number()])
            .await
            .unwrap();
        let finality = hex::decode(&finality.unwrap_or_default()["0x".len()..]).unwrap();
        let finality = FinalityProof::<GearHeader>::decode(&mut &finality[..]).unwrap();

        let justification = finality.justification;
        let justification = sp_consensus_grandpa::GrandpaJustification::<GearHeader>::decode(
            &mut &justification[..],
        )
        .unwrap();

        let set_id_address = gsdk::Api::storage_root(GrandpaStorage::CurrentSetId);
        let set_id = block
            .storage()
            .fetch(&set_id_address)
            .await
            .unwrap()
            .unwrap()
            .encoded()
            .to_vec();
        let set_id = u64::decode(&mut &*set_id).unwrap();

        let pre_commit = justification.commit.precommits[0].clone();
        assert_eq!(pre_commit.precommit.target_hash, block_hash);

        for pc in &justification.commit.precommits {
            assert_eq!(pc.precommit.target_hash, pre_commit.precommit.target_hash);
            assert_eq!(
                pc.precommit.target_number,
                pre_commit.precommit.target_number
            );
        }

        let signed_data = sp_consensus_grandpa::localized_payload(
            justification.round,
            set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(pre_commit.precommit),
        );

        // TODO: refactor
        // This trick works for now because validator set actually don't change
        // So we can treat QueuedKeys as ones used in the current session.
        let session_keys_address = gsdk::Api::storage_root(SessionStorage::QueuedKeys);
        let session_keys_data = block
            .storage()
            .fetch(&session_keys_address)
            .await
            .unwrap()
            .unwrap()
            .encoded()
            .to_vec();
        let session_keys =
            Vec::<(AccountId32, SessionKeys)>::decode(&mut &session_keys_data[..]).unwrap();
        let validator_set = session_keys
            .into_iter()
            .map(|sc| sc.1.grandpa.0 .0)
            .collect::<Vec<_>>();

        assert_eq!(VALIDATOR_COUNT, validator_set.len());

        let validator_set = validator_set.try_into().unwrap();

        assert_eq!(signed_data.len() * 8, VOTE_LENGTH_IN_BITS);

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
}
