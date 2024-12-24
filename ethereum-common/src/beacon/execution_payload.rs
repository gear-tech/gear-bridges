use super::*;

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/deneb/beacon-chain.md#executionpayload).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct ExecutionPayload {
    pub parent_hash: Bytes32,
    pub fee_recipient: Address,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub block_number: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub gas_limit: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub gas_used: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub timestamp: u64,
    pub extra_data: base_types::ByteList<32>,
    #[serde(deserialize_with = "utils::deserialize_u256")]
    pub base_fee_per_gas: U256,
    pub block_hash: Bytes32,
    pub transactions: base_types::List<Transaction, 1_048_576>,
    pub withdrawals: base_types::List<Withdrawal, 16>,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub blob_gas_used: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub excess_blob_gas: u64,
}
