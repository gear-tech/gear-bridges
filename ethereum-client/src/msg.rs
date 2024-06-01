use alloy_primitives::{Address, Bytes, U256};
use primitive_types::H256;
use std::io::Read;
/*
#[derive(Clone, Debug)]
pub struct VaraMessage {
    pub sender: H256,
    pub receiver: Address,
    pub nonce: U256,
    pub data: Bytes,
}

impl VaraMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret: Vec<u8> = Vec::with_capacity(96 + self.data.len());
        ret.extend(self.sender.as_fixed_bytes());
        ret.extend(self.receiver.to_vec());
        ret.extend(self.nonce.to_be_bytes::<32>());
        ret.extend(self.data.to_vec());
        ret
    }
}

 */
