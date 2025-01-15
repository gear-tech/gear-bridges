use super::*;

/// A homogenous collection of a fixed number of values.
///
/// NOTE: collection of length `0` is illegal.
#[derive(Clone, TypeInfo)]
pub struct FixedArray<T, const N: usize>(pub [T; N]);

impl<T: PartialEq, const N: usize> PartialEq for FixedArray<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Debug, const N: usize> fmt::Debug for FixedArray<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        if f.alternate() {
            write!(f, "FixedArray<{}, {N}>{:#?}", any::type_name::<T>(), self.0)
        } else {
            write!(f, "FixedArray<{}, {N}>{:?}", any::type_name::<T>(), self.0)
        }
    }
}

struct VectorVisitor<T>(PhantomData<Vec<T>>);

impl<'de, T: Deserialize<'de>> de::Visitor<'de> for VectorVisitor<T> {
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("array of objects")
    }

    fn visit_seq<S>(self, visitor: S) -> Result<Self::Value, S::Error>
    where
        S: de::SeqAccess<'de>,
    {
        Deserialize::deserialize(de::value::SeqAccessDeserializer::new(visitor))
    }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for FixedArray<T, N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let data = deserializer.deserialize_seq(VectorVisitor(PhantomData))?;

        Ok(Self(data.try_into().map_err(|_| {
            de::Error::custom("unable to create [T; N] from Vec<T>")
        })?))
    }
}

impl<T: TreeHash, const N: usize> TreeHash for FixedArray<T, N> {
    fn tree_hash_type() -> TreeHashType {
        TreeHashType::Vector
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("Vector should never be packed.")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("Vector should never be packed.")
    }

    fn tree_hash_root(&self) -> Hash256 {
        utils::vec_tree_hash_root::<T, N>(&self.0)
    }
}

impl<T: Decode, const N: usize> Decode for FixedArray<T, N> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        let data = <[T; N] as Decode>::decode(input)?;

        Ok(Self(data))
    }
}

impl<T: Encode, const N: usize> Encode for FixedArray<T, N> {
    fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }

    fn encode_to<W: parity_scale_codec::Output + ?Sized>(&self, dest: &mut W) {
        self.0.encode_to(dest)
    }

    fn encoded_size(&self) -> usize {
        self.0.encoded_size()
    }

    fn size_hint(&self) -> usize {
        self.0.size_hint()
    }

    fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        self.0.using_encoded(f)
    }
}
