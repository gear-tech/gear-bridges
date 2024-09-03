use gear_rpc_client::dto::Message;
use primitive_types::H256;

pub mod block_listener;
pub mod merkle_root_listener;
pub mod message_paid_listener;
pub mod message_queued_listener;
pub mod message_sender;

type AuthoritySetId = u64;

pub struct MessageInBlock {
    pub message: Message,
    pub block: u32,
    pub block_hash: H256,
}
