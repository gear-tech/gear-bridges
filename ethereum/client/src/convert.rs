use alloy_primitives::{Address, Bytes as aBytes, Uint, B256 as aB256, U256 as aU256};
use primitive_types::{H256 as eH256, U256 as eU256};

pub trait Convert<T> {
    fn convert(self) -> T;
}

impl<T> Convert<T> for T {
    fn convert(self) -> T {
        self
    }
}

impl Convert<Address> for [u8; 20] {
    fn convert(self) -> Address {
        Address::from_slice(&self)
    }
}

impl Convert<Uint<256, 4>> for u128 {
    fn convert(self) -> Uint<256, 4> {
        Uint::<256, 4>::from(self)
    }
}

impl Convert<aU256> for u32 {
    fn convert(self) -> aU256 {
        let bytes = &self.to_be_bytes()[..];
        let bytes = [&[0; 32 - 4], bytes].concat();
        aU256::from_be_bytes::<32>(bytes.try_into().expect("Incorrect amount of padding bytes"))
    }
}

impl Convert<aU256> for eU256 {
    fn convert(self) -> aU256 {
        let mut be = [0u8; 32];
        self.to_big_endian(&mut be);
        aU256::from_be_bytes::<32>(be)
    }
}

impl Convert<aU256> for [u8; 32] {
    fn convert(self) -> aU256 {
        aU256::from_be_bytes::<32>(self)
    }
}

impl Convert<aBytes> for &[u8] {
    fn convert(self) -> aBytes {
        aBytes::copy_from_slice(self)
    }
}

impl Convert<aB256> for [u8; 32] {
    fn convert(self) -> aB256 {
        aB256::new(self)
    }
}

impl Convert<aB256> for eH256 {
    fn convert(self) -> aB256 {
        aB256::from_slice(self.as_fixed_bytes())
    }
}
