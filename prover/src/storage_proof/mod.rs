mod node_parser;
mod storage_address;

#[cfg(test)]
mod tests_common {
    pub fn pad_byte_vec<const L: usize>(data: Vec<u8>) -> [u8; L] {
        assert!(data.len() <= L);
        data.into_iter()
            .chain(std::iter::repeat(0))
            .take(L)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}
