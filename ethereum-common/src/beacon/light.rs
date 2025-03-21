//! Module contains lightened versions of the entities, i.e. some fields
//! are replaced by its tree hash root value.

use super::*;

#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct ExecutionPayload {
    pub parent_hash: Bytes32,
    pub fee_recipient: Address,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: H256,
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
    pub transactions: H256,
    pub withdrawals: H256,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub blob_gas_used: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub excess_blob_gas: u64,
}

impl From<super::ExecutionPayload> for ExecutionPayload {
    fn from(value: super::ExecutionPayload) -> Self {
        Self {
            parent_hash: value.parent_hash,
            fee_recipient: value.fee_recipient,
            state_root: value.state_root,
            receipts_root: value.receipts_root,
            logs_bloom: value.logs_bloom.tree_hash_root(),
            prev_randao: value.prev_randao,
            block_number: value.block_number,
            gas_limit: value.gas_limit,
            gas_used: value.gas_used,
            timestamp: value.timestamp,
            extra_data: value.extra_data,
            base_fee_per_gas: value.base_fee_per_gas,
            block_hash: value.block_hash,
            transactions: value.transactions.tree_hash_root(),
            withdrawals: value.withdrawals.tree_hash_root(),
            blob_gas_used: value.blob_gas_used,
            excess_blob_gas: value.excess_blob_gas,
        }
    }
}

#[derive(
    Debug, Clone, Decode, Encode, PartialEq, tree_hash_derive::TreeHash, Deserialize, TypeInfo,
)]
pub struct BlockBody {
    pub randao_reveal: H256,
    pub eth1_data: H256,
    pub graffiti: Bytes32,
    pub proposer_slashings: H256,
    pub attester_slashings: H256,
    pub attestations: H256,
    pub deposits: H256,
    pub voluntary_exits: H256,
    pub sync_aggregate: H256,
    pub execution_payload: ExecutionPayload,
    pub bls_to_execution_changes: H256,
    pub blob_kzg_commitments: H256,
}

impl From<super::BlockBody> for BlockBody {
    fn from(value: super::BlockBody) -> Self {
        Self {
            randao_reveal: value.randao_reveal.tree_hash_root(),
            eth1_data: value.eth1_data.tree_hash_root(),
            graffiti: value.graffiti,
            proposer_slashings: value.proposer_slashings.tree_hash_root(),
            attester_slashings: value.attester_slashings.tree_hash_root(),
            attestations: value.attestations.tree_hash_root(),
            deposits: value.deposits.tree_hash_root(),
            voluntary_exits: value.voluntary_exits.tree_hash_root(),
            sync_aggregate: value.sync_aggregate.tree_hash_root(),
            execution_payload: value.execution_payload.into(),
            bls_to_execution_changes: value.bls_to_execution_changes.tree_hash_root(),
            blob_kzg_commitments: value.blob_kzg_commitments.tree_hash_root(),
        }
    }
}

#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct BlockGeneric<BlockBody: tree_hash::TreeHash> {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub slot: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub proposer_index: u64,
    pub parent_root: Hash256,
    pub state_root: Hash256,
    pub body: BlockBody,
}

impl<BlockBody, BlockBodyLight> From<beacon::block::BlockGeneric<BlockBody>>
    for BlockGeneric<BlockBodyLight>
where
    BlockBody: tree_hash::TreeHash,
    BlockBodyLight: From<BlockBody> + tree_hash::TreeHash,
{
    fn from(value: beacon::block::BlockGeneric<BlockBody>) -> Self {
        Self {
            slot: value.slot,
            proposer_index: value.proposer_index,
            parent_root: value.parent_root,
            state_root: value.state_root,
            body: value.body.into(),
        }
    }
}

pub type Block = BlockGeneric<BlockBody>;

pub mod electra {
    use super::*;
    use crate::beacon::electra;

    #[derive(
        Debug, Clone, Decode, Encode, PartialEq, tree_hash_derive::TreeHash, Deserialize, TypeInfo,
    )]
    pub struct BlockBody {
        pub randao_reveal: H256,
        pub eth1_data: H256,
        pub graffiti: Bytes32,
        pub proposer_slashings: H256,
        pub attester_slashings: H256,
        pub attestations: H256,
        pub deposits: H256,
        pub voluntary_exits: H256,
        pub sync_aggregate: H256,
        pub execution_payload: super::ExecutionPayload,
        pub bls_to_execution_changes: H256,
        pub blob_kzg_commitments: H256,
        pub execution_requests: H256,
    }

    impl From<electra::BlockBody> for BlockBody {
        fn from(value: electra::BlockBody) -> Self {
            Self {
                randao_reveal: value.randao_reveal.tree_hash_root(),
                eth1_data: value.eth1_data.tree_hash_root(),
                graffiti: value.graffiti,
                proposer_slashings: value.proposer_slashings.tree_hash_root(),
                attester_slashings: value.attester_slashings.tree_hash_root(),
                attestations: value.attestations.tree_hash_root(),
                deposits: value.deposits.tree_hash_root(),
                voluntary_exits: value.voluntary_exits.tree_hash_root(),
                sync_aggregate: value.sync_aggregate.tree_hash_root(),
                execution_payload: value.execution_payload.into(),
                bls_to_execution_changes: value.bls_to_execution_changes.tree_hash_root(),
                blob_kzg_commitments: value.blob_kzg_commitments.tree_hash_root(),
                execution_requests: value.execution_requests.tree_hash_root(),
            }
        }
    }

    pub type Block = super::BlockGeneric<BlockBody>;
}
