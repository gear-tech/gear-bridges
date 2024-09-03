use gear_rpc_client::dto::Message;
use primitive_types::H256;

pub mod ethereum_message_sender;
pub mod gear_block_listener;
pub mod merkle_root_listener;
pub mod message_paid_event_extractor;
pub mod message_queued_event_extractor;
pub mod paid_messages_filter;

type AuthoritySetId = u64;

pub struct MessageInBlock {
    pub message: Message,
    pub block: u32,
    pub block_hash: H256,
}
