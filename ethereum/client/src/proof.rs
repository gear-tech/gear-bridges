use alloy::primitives::{Bytes, B256, U256};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockMerkleRootProof {
    pub proof: Bytes,
    pub block_number: U256,
    pub merkle_root: B256,
}

impl BlockMerkleRootProof {
    pub fn try_from_json_string(data: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(data)
    }
}
