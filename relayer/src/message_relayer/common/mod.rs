use ethereum_client::TxHash;
use gear_rpc_client::dto::Message;
use primitive_types::H256;

pub mod ethereum;
pub mod gear;
pub mod paid_messages_filter;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
pub struct AuthoritySetId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
pub struct GearBlockNumber(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, derive_more::Display)]
pub struct EthereumBlockNumber(pub u64);

#[derive(
    Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default, derive_more::Display,
)]
pub struct EthereumSlotNumber(pub u64);

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug)]
pub struct RelayedMerkleRoot {
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub authority_set_id: AuthoritySetId,
    pub merkle_root: H256,
}

#[derive(Clone, Debug)]
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
