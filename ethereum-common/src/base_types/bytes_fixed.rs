use super::*;

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
pub struct BytesFixed<const N: usize>(pub FixedArray<u8, N>);

impl<const N: usize> AsRef<[u8]> for BytesFixed<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0 .0
    }
}

impl<const N: usize> From<[u8; N]> for BytesFixed<N> {
    fn from(value: [u8; N]) -> Self {
        Self(FixedArray(value))
    }
}

impl<const N: usize> TreeHash for BytesFixed<N> {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        FixedArray::<u8, N>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        self.0.tree_hash_packed_encoding()
    }

    fn tree_hash_packing_factor() -> usize {
        FixedArray::<u8, N>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> Hash256 {
        self.0.tree_hash_root()
    }
}

impl<'de, const N: usize> Deserialize<'de> for BytesFixed<N> {
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = utils::decode_hex_bytes(deserializer)?;

        Ok(Self(FixedArray(bytes.try_into().map_err(|_| {
            <D::Error as de::Error>::custom("Failed to convert to BytesFixed")
        })?)))
    }
}
