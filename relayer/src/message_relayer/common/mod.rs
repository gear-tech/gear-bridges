use ethereum_client::TxHash;
use ethereum_common::Hash256;
use gear_rpc_client::dto::{MerkleProof, Message};
use gsdk::{
    config::Header,
    metadata::{
        gear::Event as GearEvent,
        gear_eth_bridge::Event as GearEthBridgeEvent,
        runtime_types::{gear_core::message::user::UserMessage, gprimitives::ActorId},
    },
    subscription::BlockEvents,
    GearConfig,
};
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use subxt::{blocks::Block, config::Header as _, OnlineClient};

pub mod ethereum;
pub mod gear;
pub mod paid_messages_filter;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
pub struct AuthoritySetId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelayedMerkleRoot {
    pub block: GearBlockNumber,
    pub block_hash: H256,
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
    pub vara_rpc_retries: u8,
}

pub struct Data {
    pub message: MessageInBlock,
    pub relayed_root: RelayedMerkleRoot,
    pub proof: MerkleProof,
}

#[derive(Clone, Debug)]
pub struct GearBlock {
    pub header: Header,
    pub events: Vec<gsdk::Event>,
}

impl GearBlock {
    pub fn new(header: Header, events: Vec<gsdk::Event>) -> Self {
        Self { header, events }
    }

    pub fn number(&self) -> u32 {
        self.header.number()
    }

    pub fn hash(&self) -> Hash256 {
        self.header.hash()
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
            gclient::Event::Gear(GearEvent::UserMessageSent {
                message:
                    UserMessage {
                        source,
                        destination,
                        payload,
                        ..
                    },
                ..
            }) if source == &ActorId(from_program.0) && destination == &ActorId(to_user.0) => {
                Some(payload.0.as_ref())
            }
            _ => None,
        })
    }

    pub async fn from_subxt_block(
        block: Block<GearConfig, OnlineClient<GearConfig>>,
    ) -> anyhow::Result<Self> {
        let header = block.header().clone();
        let events = BlockEvents::new(block).await?;
        let events = events.events()?;
        Ok(Self::new(header, events))
    }
}

fn message_queued_events_of(
    block: &GearBlock,
) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
    block.events().iter().filter_map(|event| match event {
        gclient::Event::GearEthBridge(GearEthBridgeEvent::MessageQueued { message, .. }) => {
            let mut nonce_le = [0; 32];
            primitive_types::U256(message.nonce.0).to_little_endian(&mut nonce_le);

            Some(gear_rpc_client::dto::Message {
                nonce_le,
                source: message.source.0,
                destination: message.destination.0,
                payload: message.payload.clone(),
            })
        }
        _ => None,
    })
}

pub mod web_request {
    use super::*;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Message {
        pub block: u32,
        pub nonce: U256,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct Messages {
        pub messages: Vec<Message>,
    }
}
