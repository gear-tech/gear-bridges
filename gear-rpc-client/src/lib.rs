use gsdk::metadata::storage::{BabeStorage, GrandpaStorage};
use parity_scale_codec::{Decode, Encode};
use prover::merkle_proof::{MerkleProof, TrieNodeData};
use sc_consensus_grandpa::FinalityProof;
use sp_core::crypto::Wraps;
use subxt::{ext::sp_core::H256, rpc_params};
use trie_db::{node::NodeHandle, ChildReference};

type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

pub struct GearApi {
    api: gsdk::Api,
}

impl GearApi {
    pub async fn new() -> GearApi {
        GearApi {
            api: gsdk::Api::new(Some("wss://archive-rpc.vara-network.io:443"))
                .await
                .unwrap(),
        }
    }

    pub async fn latest_finalized_block(&self) -> H256 {
        self.api.rpc().finalized_head().await.unwrap()
    }

    pub async fn fetch_justification(
        &self,
        block: H256,
    ) -> prover::block_justification::BlockJustification {
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

        let signed_data = sp_consensus_grandpa::localized_payload(
            justification.round,
            set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(pre_commit.precommit),
        );

        prover::block_justification::BlockJustification {
            msg: signed_data,
            pre_commits: justification
                .commit
                .precommits
                .into_iter()
                .map(|pc| prover::block_justification::PreCommit {
                    public_key: pc.id.as_inner_ref().as_array_ref().to_owned(),
                    signature: pc.signature.as_inner_ref().0.to_owned(),
                })
                .collect(),
        }
    }

    pub async fn fetch_next_authorities(&self, block: H256) {
        let block = (*self.api).blocks().at(block).await.unwrap();
        let storage = block.storage();

        let address = gsdk::Api::storage_root(BabeStorage::Authorities);
        let authorities = storage.fetch(&address).await.unwrap();
        let authorities = Vec::<(
            pallet_babe::AuthorityId,
            sp_consensus_babe::BabeAuthorityWeight,
        )>::decode(&mut authorities.unwrap().encoded())
        .unwrap();

        let address = gsdk::Api::storage_root(BabeStorage::NextAuthorities);
        let next_authorities = storage.fetch(&address).await.unwrap();
        let next_authorities = Vec::<(
            pallet_babe::AuthorityId,
            sp_consensus_babe::BabeAuthorityWeight,
        )>::decode(&mut next_authorities.unwrap().encoded())
        .unwrap();

        println!("AUTH: {} {}", authorities.len(), next_authorities.len());
    }

    pub async fn fetch_next_authorities_merkle_proof(&self, block: H256) -> MerkleProof {
        let address = gsdk::Api::storage_root(BabeStorage::NextAuthorities).to_root_bytes();
        self.fetch_merkle_proof(block, &address).await
    }

    async fn fetch_merkle_proof(&self, block: H256, address: &[u8]) -> MerkleProof {
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
            .read_proof(storage_keys.clone(), Some(block.hash()))
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
            assert!(matches!(value.clone(), Value::Inline(b) if b.len() == 0));
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

        MerkleProof {
            nodes,
            leaf_data: storage_data,
            root_hash: state_root.0,
        }
    }
}
