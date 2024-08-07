use super::*;

#[derive(Debug, Clone, tree_hash_derive::TreeHash, Decode, Encode, Deserialize, TypeInfo)]
pub struct BlockHeader {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub slot: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub proposer_index: u64,
    pub parent_root: Hash256,
    pub state_root: Hash256,
    pub body_root: Hash256,
}
