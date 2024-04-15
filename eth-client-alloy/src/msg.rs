use alloy_primitives::{Address, Bytes, U256};
use primitive_types::H256;
use std::io::Read;

#[derive(Clone, Debug)]
pub struct VaraMessage {
    pub vara_address: H256,
    pub eth_address: Address,
    pub nonce: U256,
    pub data: Bytes,
}

impl VaraMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret: Vec<u8> = Vec::with_capacity(96 + self.data.len());
        ret.extend(self.vara_address.as_fixed_bytes());
        ret.extend(self.eth_address.to_vec());
        //ret.extend(self.eth_address.as_fixed_bytes());
        ret.extend(self.nonce.to_be_bytes::<32>());
        ret.extend(self.data.to_vec());
        ret
    }
}

/*
impl AsRef<[u8]> for ContentMessage {
    fn as_ref(&self) -> &[u8] {
        &self.buf

        /*let mut ret: Vec<u8>;
        ret = self.eth_address.as_fixed_bytes().to_vec();

        ret.append(&mut self.vara_address.as_fixed_bytes().to_vec());
        ret.append(&mut self.nonce.to_be_bytes::<32>().to_vec());
        ret.append(&mut self.data.to_vec());
        ret.as_slice()

         */
    }
}

 */
