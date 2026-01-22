use ethereum_client::TxHash;
use gear_rpc_client::{dto::Message, GearApi, GearHeader};
use gsdk::{config::Header, GearConfig};
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use sp_consensus_grandpa::GrandpaJustification;
use subxt::{
    blocks::Block,
    config::{substrate::BlakeTwo256, Header as _},
    OnlineClient,
};

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
    pub vara_endpoint: String,
}

#[derive(Clone, Debug)]
pub struct GearBlock {
    pub header: Header,
    pub events: Vec<gsdk::Event>,
    pub grandpa_justification: GrandpaJustification<GearHeader>,
}

impl GearBlock {
    pub fn new(
        header: Header,
        events: Vec<gsdk::Event>,
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

    pub fn hash(&self) -> H256 {
        let blake2_hasher = BlakeTwo256;
        self.header.hash_with(blake2_hasher).0.into()
    }

    pub fn events(&self) -> &[gsdk::Event] {
        &self.events
    }

    pub fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
    ) -> impl Iterator<Item = &[u8]> + use<'_> {
        self.events.iter().filter_map(move |event| match event {
            gsdk::Event::Gear(gsdk::gear::gear::Event::UserMessageSent { message, .. })
                if message.source().into_bytes() == from_program.0
                    && message.destination().into_bytes() == to_user.0 =>
            {
                Some(message.payload_bytes())
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
            .at(primitive_types::H256::from(
                justification.commit.target_hash.0,
            ))
            .await?;

        let header = block.header().clone();
        let events = api.get_events_at(Some(block.hash())).await?;

        Ok(Self::new(header, events, justification))
    }
}

fn message_queued_events_of(
    block: &GearBlock,
) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
    block.events().iter().filter_map(|event| match event {
        gsdk::Event::GearEthBridge(gsdk::gear::gear_eth_bridge::Event::MessageQueued {
            message,
            ..
        }) => {
            let nonce_be = primitive_types::U256(message.nonce.0).to_big_endian();

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
