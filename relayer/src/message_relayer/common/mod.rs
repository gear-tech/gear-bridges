use ethereum_client::TxHash;
use ethereum_common::Hash256;
use gear_rpc_client::{
    dto::{Message, PreCommit},
    ext::sp_core::crypto::Wraps,
    metadata::runtime_types::{gear_core::message::user::UserMessage, gprimitives::ActorId},
    GearApi, GearHeader,
};
use gsdk::{config::Header, GearConfig};
use primitive_types::{H256, U256};
use prover::consts::ED25519_PUBLIC_KEY_SIZE;
use serde::{Deserialize, Serialize};
use sp_consensus_grandpa::GrandpaJustification;
use subxt::{blocks::Block, config::Header as _, OnlineClient};

pub mod ethereum;
pub mod gear;
pub mod paid_messages_filter;

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct AuthoritySetId(pub u64);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct GearBlockNumber(pub u32);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct EthereumBlockNumber(pub u64);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    Default,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct EthereumSlotNumber(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageInBlock {
    pub message: Message,
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub authority_set_id: AuthoritySetId,
}

#[derive(Clone, Copy, Debug)]
pub struct PaidMessage {
    pub nonce: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelayedMerkleRoot {
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub timestamp: u64,
    pub authority_set_id: AuthoritySetId,
    pub merkle_root: H256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxHashWithSlot {
    pub slot_number: EthereumSlotNumber,
    pub tx_hash: TxHash,
}

#[derive(Clone, Debug)]
pub struct GSdkArgs {
    pub vara_domain: String,
    pub vara_port: u16,
}

#[derive(Clone, Debug)]
pub struct GearBlock {
    pub header: Header,
    pub events: Vec<gear_rpc_client::metadata::Event>,
    pub grandpa_justification: GrandpaJustification<GearHeader>,
}

impl GearBlock {
    pub fn new(
        header: Header,
        events: Vec<gear_rpc_client::metadata::Event>,
        grandpa_justification: GrandpaJustification<GearHeader>,
    ) -> Self {
        Self {
            header,
            events,
            grandpa_justification,
        }
    }

    pub fn number(&self) -> u32 {
        self.header.number()
    }

    pub fn hash(&self) -> Hash256 {
        self.header.hash()
    }

    pub fn events(&self) -> &[gear_rpc_client::metadata::Event] {
        &self.events
    }

    pub fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
    ) -> impl Iterator<Item = &[u8]> + use<'_> {
        self.events.iter().filter_map(move |event| match event {
            gear_rpc_client::metadata::Event::Gear(
                gear_rpc_client::metadata::gear::Event::UserMessageSent {
                    message:
                        UserMessage {
                            source,
                            destination,
                            payload,
                            ..
                        },
                    ..
                },
            ) if source == &ActorId(from_program.0) && destination == &ActorId(to_user.0) => {
                Some(payload.0.as_ref())
            }
            _ => None,
        })
    }

    pub async fn from_subxt_block(
        api: &GearApi,
        block: Block<GearConfig, OnlineClient<GearConfig>>,
    ) -> anyhow::Result<Self> {
        let justification = api.get_justification(block.hash()).await?;
        let header = block.header().clone();
        let events = api.get_events_at(Some(block.hash())).await?;

        Ok(Self::new(header, events, justification))
    }

    pub async fn from_justification(
        api: &GearApi,
        justification: GrandpaJustification<GearHeader>,
    ) -> anyhow::Result<Self> {
        let block = api
            .api
            .blocks()
            .at(justification.commit.target_hash)
            .await?;

        let header = block.header().clone();
        let events = api.get_events_at(Some(block.hash())).await?;

        Ok(Self::new(header, events, justification))
    }

    /// Produce a raw block inclusion proof from the block's grandpa justification.
    pub async fn inclusion_proof(&self, api: &GearApi) -> anyhow::Result<RawBlockInclusionProof> {
        let required_authority_set_id = api.signed_by_authority_set_id(self.hash()).await?;
        let validator_set = api.fetch_authority_set(required_authority_set_id).await?;
        let pre_commits: Vec<_> = self
            .grandpa_justification
            .commit
            .precommits
            .iter()
            .map(|pc| gear_rpc_client::dto::PreCommit {
                public_key: pc.id.as_inner_ref().as_array_ref().to_owned(),
                signature: pc.signature.as_inner_ref().0.to_owned(),
            })
            .collect();

        Ok(RawBlockInclusionProof {
            justification_round: self.grandpa_justification.round,
            required_authority_set_id,
            precommit: (
                self.grandpa_justification.commit.target_hash,
                self.grandpa_justification.commit.target_number,
            ),
            validator_set,
            block_hash: self.hash(),
            block_number: self.number(),
            pre_commits,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RawBlockInclusionProof {
    pub justification_round: u64,
    pub required_authority_set_id: u64,
    pub precommit: (H256, u32),
    pub validator_set: Vec<[u8; ED25519_PUBLIC_KEY_SIZE]>,
    pub block_hash: H256,
    pub block_number: u32,
    pub pre_commits: Vec<PreCommit>,
}

/* manual serialization to hex encode some fields */
impl Serialize for RawBlockInclusionProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("RawBlockInclusionProof", 7)?;
        state.serialize_field("justification_round", &self.justification_round)?;
        state.serialize_field("required_authority_set_id", &self.required_authority_set_id)?;

        let precommit = (hex::encode(self.precommit.0.as_bytes()), self.precommit.1);
        state.serialize_field("precommit", &precommit)?;

        let validator_set: Vec<String> = self
            .validator_set
            .iter()
            .map(|pk| hex::encode(pk))
            .collect();
        state.serialize_field("validator_set", &validator_set)?;

        state.serialize_field("block_hash", &hex::encode(self.block_hash.as_bytes()))?;
        state.serialize_field("block_number", &self.block_number)?;
        state.serialize_field("pre_commits", &self.pre_commits)?;
        state.end()
    }
}

/* manual deserialization because some fields are hex encoded */
impl<'de> Deserialize<'de> for RawBlockInclusionProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            justification_round: u64,
            required_authority_set_id: u64,
            precommit: (String, u32),
            validator_set: Vec<String>,
            block_hash: String,
            block_number: u32,
            pre_commits: Vec<PreCommit>,
        }

        let helper = Helper::deserialize(deserializer)?;

        let precommit_hash_bytes =
            hex::decode(helper.precommit.0).map_err(serde::de::Error::custom)?;
        if precommit_hash_bytes.len() != 32 {
            return Err(serde::de::Error::custom("precommit hash must be 32 bytes"));
        }
        let precommit_hash = H256::from_slice(&precommit_hash_bytes);

        let mut validator_set = Vec::with_capacity(helper.validator_set.len());
        for s in helper.validator_set {
            let pk = hex::decode(s).map_err(serde::de::Error::custom)?;
            if pk.len() != ED25519_PUBLIC_KEY_SIZE {
                return Err(serde::de::Error::custom(
                    "validator_set entry has wrong length",
                ));
            }
            let mut arr = [0u8; ED25519_PUBLIC_KEY_SIZE];
            arr.copy_from_slice(&pk);
            validator_set.push(arr);
        }

        let block_hash_bytes = hex::decode(helper.block_hash).map_err(serde::de::Error::custom)?;
        if block_hash_bytes.len() != 32 {
            return Err(serde::de::Error::custom("block_hash must be 32 bytes"));
        }
        let block_hash = H256::from_slice(&block_hash_bytes);

        Ok(Self {
            justification_round: helper.justification_round,
            required_authority_set_id: helper.required_authority_set_id,
            precommit: (precommit_hash, helper.precommit.1),
            validator_set,
            block_hash,
            block_number: helper.block_number,
            pre_commits: helper.pre_commits,
        })
    }
}

impl Into<(H256, gear_rpc_client::dto::BlockFinalityProof)> for RawBlockInclusionProof {
    fn into(self) -> (H256, gear_rpc_client::dto::BlockFinalityProof) {
        let signed_data = sp_consensus_grandpa::localized_payload(
            self.justification_round,
            self.required_authority_set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(
                sp_consensus_grandpa::Precommit::<GearHeader>::new(
                    self.precommit.0,
                    self.precommit.1,
                ),
            ),
        );

        (
            self.block_hash,
            gear_rpc_client::dto::BlockFinalityProof {
                validator_set: self.validator_set,
                message: signed_data,
                pre_commits: self.pre_commits,
            },
        )
    }
}

fn message_queued_events_of(
    block: &GearBlock,
) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
    block.events().iter().filter_map(|event| match event {
        gear_rpc_client::metadata::Event::GearEthBridge(
            gear_rpc_client::metadata::gear_eth_bridge::Event::MessageQueued { message, .. },
        ) => {
            let mut nonce_be = [0; 32];
            primitive_types::U256(message.nonce.0).to_big_endian(&mut nonce_be);

            Some(gear_rpc_client::dto::Message {
                nonce_be,
                source: message.source.0,
                destination: message.destination.0,
                payload: message.payload.clone(),
            })
        }
        _ => None,
    })
}

pub fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_be.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_hash::keccak_256(&data, &mut hash);

    hash
}

pub mod web_request {
    use super::*;
    use tokio::sync::oneshot::Sender;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Message {
        pub block: u32,
        pub nonce: U256,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct Messages {
        pub messages: Vec<Message>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct MerkleRootBlocks {
        pub blocks: Vec<u32>,
    }

    pub enum MerkleRootsRequest {
        GetMerkleRootProof {
            block_number: u32,
            response: Sender<MerkleRootsResponse>,
        },
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum MerkleRootsResponse {
        MerkleRootProof {
            proof: Vec<u8>,
            proof_block_number: u32,
            merkle_root: H256,
            block_number: u32,
            block_hash: H256,
        },

        NoMerkleRootOnBlock {
            block_number: u32,
        },

        Failed {
            message: String,
        },
    }
}

#[cfg(test)]
mod tests {
    use prover::consts::ED25519_SIGNATURE_SIZE;

    use super::*;

    #[test]
    fn raw_block_inclusion_proof_hex_serde_roundtrip() {
        let p = RawBlockInclusionProof {
            justification_round: 42,
            required_authority_set_id: 7,
            precommit: (H256::from_low_u64_be(0x11), 123),
            validator_set: vec![
                [0xAB; ED25519_PUBLIC_KEY_SIZE],
                [0xCD; ED25519_PUBLIC_KEY_SIZE],
            ],
            block_hash: H256::from_low_u64_be(0x22),
            block_number: 999,
            pre_commits: vec![PreCommit {
                public_key: [0xEF; ED25519_PUBLIC_KEY_SIZE],
                signature: [0x01; ED25519_SIGNATURE_SIZE],
            }],
        };

        let json = serde_json::to_string(&p).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json value");

        // Ensure these are encoded as hex strings (not arrays of ints).
        assert!(v.get("block_hash").unwrap().as_str().is_some());
        assert!(v.get("validator_set").unwrap().as_array().unwrap()[0]
            .as_str()
            .is_some());
        assert!(v.get("precommit").unwrap().as_array().unwrap()[0]
            .as_str()
            .is_some());

        let decoded: RawBlockInclusionProof = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.justification_round, p.justification_round);
        assert_eq!(
            decoded.required_authority_set_id,
            p.required_authority_set_id
        );
        assert_eq!(decoded.precommit, p.precommit);
        assert_eq!(decoded.validator_set, p.validator_set);
        assert_eq!(decoded.block_hash, p.block_hash);
        assert_eq!(decoded.block_number, p.block_number);
        assert_eq!(decoded.pre_commits.len(), p.pre_commits.len());
    }
}
