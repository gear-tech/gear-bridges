use gear_rpc_client::dto::Message;
use primitive_types::H256;

pub mod ethereum_message_sender;
pub mod gear_block_listener;
pub mod merkle_root_listener;
pub mod message_paid_event_extractor;
pub mod message_queued_event_extractor;
pub mod paid_messages_filter;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
pub struct AuthoritySetId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]

pub struct GearBlockNumber(pub u32);

#[derive(Clone, Debug)]
pub struct MessageInBlock {
    pub message: Message,
    pub block: GearBlockNumber,
    pub block_hash: H256,
}

#[derive(Clone, Copy, Debug)]
pub struct PaidMessage {
    pub nonce: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
pub struct RelayedMerkleRoot {
    pub block: GearBlockNumber,
    pub authority_set_id: AuthoritySetId,
}
