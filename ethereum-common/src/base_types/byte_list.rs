use super::*;

#[derive(Debug, Clone, Decode, Encode, TypeInfo)]
pub struct ByteList<const N: usize>(pub List<u8, N>);

impl<const N: usize> TryFrom<&[u8]> for ByteList<N> {
    type Error = String;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(data.try_into()?))
    }
}

impl<const N: usize> TreeHash for ByteList<N> {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        List::<u8, N>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        self.0.tree_hash_packed_encoding()
    }

    fn tree_hash_packing_factor() -> usize {
        List::<u8, N>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> Hash256 {
        self.0.tree_hash_root()
    }
}

impl<'de, const N: usize> Deserialize<'de> for ByteList<N> {
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = utils::decode_hex_bytes(deserializer)?;

        Ok(Self(bytes.try_into().map_err(|_| {
            <D::Error as de::Error>::custom("Failed to convert to BytesFixed")
        })?))
    }
}
