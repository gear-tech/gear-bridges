use ethereum_client::TxHash;
use gear_rpc_client::dto::Message;
use primitive_types::{H160, H256, U256};

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

#[derive(Clone, Debug)]
pub struct ERC20Deposit {
    pub data: ERC20DepositData,
    pub tx: ERC20DepositTx,
}

#[derive(Clone, Debug)]
pub struct ERC20DepositData {
    pub from: H160,
    pub to: H256,
    pub token: H160,
    pub amount: U256,
}

#[derive(Clone, Debug)]
pub struct ERC20DepositTx {
    pub slot_number: EthereumSlotNumber,
    pub tx_hash: TxHash,
}
