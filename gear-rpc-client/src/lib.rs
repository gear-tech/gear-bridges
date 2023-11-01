use gsdk::metadata::storage::{BabeStorage, GrandpaStorage};
use parity_scale_codec::Decode;
use sc_consensus_grandpa::FinalityProof;
use sp_core::crypto::Wraps;
use subxt::{ext::sp_core::H256, rpc_params};

type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

pub struct GearApi {
    api: gsdk::Api,
}

impl GearApi {
    pub async fn new() -> GearApi {
        GearApi {
            api: gsdk::Api::new(Some("wss://rpc.vara-network.io:443"))
                .await
                .unwrap(),
        }
    }

    pub async fn latest_finalized_block(&self) -> H256 {
        self.api.rpc().finalized_head().await.unwrap()
    }

    pub async fn fetch_inclusion_proof(&self, block: H256) {
        let block = (*self.api).blocks().at(block).await.unwrap();

        let address = pallet_grandpa::fg_primitives::GRANDPA_AUTHORITIES_KEY;

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

        let trie_proof = sp_trie::generate_trie_proof::<
            sp_trie::LayoutV1<sp_core::Blake2Hasher>,
            _,
            _,
            _,
        >(&storage_proof, state_root, storage_keys.iter())
        .unwrap();

        let valid: Result<(), sp_trie::VerifyError<H256, sp_trie::Error<H256>>> =
            trie_db::proof::verify_proof::<sp_trie::LayoutV1<sp_core::Blake2Hasher>, _, _, _>(
                &state_root,
                &trie_proof,
                [&(address.to_vec(), Some(storage_data))],
            );

        println!("proof validity: {}", valid.is_ok());
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

    pub async fn fetch_authorities(&self, block: H256) {
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
}
