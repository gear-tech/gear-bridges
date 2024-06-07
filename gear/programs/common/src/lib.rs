use primitive_types::{H160, U256};

pub struct VARA2ETHRequest {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

impl VARA2ETHRequest {
    pub fn pack(self) -> Vec<u8> {
        const U256_BYTES: usize = 256 / 8;
        const AMOUNT_OFFSET: usize = 160 / 8 + 160 / 8;

        let mut packed = [
            self.receiver.as_bytes(),
            self.token_id.as_bytes(),
            &[0; U256_BYTES],
        ]
        .concat();

        self.amount.to_big_endian(&mut packed[AMOUNT_OFFSET..]);

        packed
    }
}
