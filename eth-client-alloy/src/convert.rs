use primitive_types::{U256 as eU256};
use alloy_primitives::{U256 as aU256, Bytes as aBytes};


pub trait Convert<T> {
    fn convert(self)->T;
}

impl Convert<aU256> for eU256{
    fn convert(self) -> aU256 {
        let mut be = [0u8;32];
        self.to_big_endian(&mut be);
        aU256::from_be_bytes::<32>(be)
    }
}


impl Convert<aU256> for aU256{
    fn convert(self) -> aU256 {
        self
    }
}

impl Convert<aBytes> for aBytes{
    fn convert(self) -> aBytes {
        self
    }
}

impl Convert<aBytes> for &[u8]{
    fn convert(self) -> aBytes {
        aBytes::copy_from_slice(self)
    }
}