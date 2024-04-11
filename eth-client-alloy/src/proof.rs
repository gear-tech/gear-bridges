use crate::abi::ContentMessage;
use alloy_primitives::{Bytes, U256};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proof {
    pub proof: Bytes,
    pub public_inputs: Vec<U256>,
}

impl Proof {
    pub fn try_from_json_string(data: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(data)
    }
}
