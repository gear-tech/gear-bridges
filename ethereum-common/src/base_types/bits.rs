use super::*;
use bitvec::{
    field::BitField,
    prelude::{BitVec, Lsb0},
};

const BITS_PER_BYTE: usize = 8;

/// Returns the minimum required bytes to represent a given number of bits.
///
/// `bit_len == 0` requires a single byte.
fn bytes_for_bit_len(bit_len: usize) -> usize {
    core::cmp::max(1, (bit_len + BITS_PER_BYTE - 1) / BITS_PER_BYTE)
}

type Inner = BitVec<u8, Lsb0>;

/// A homogenous collection of a fixed number of boolean values.
///
/// NOTE: a `Vector` of length `0` is illegal.
#[derive(PartialEq, Eq, Clone)]
pub struct Vector<const N: usize>(Inner);

impl<'de, const N: usize> Deserialize<'de> for Vector<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = utils::decode_hex_bytes(deserializer)?;

        Self::from_bytes(&bytes).map_err(<D::Error as de::Error>::custom)
    }
}

impl<const N: usize> fmt::Debug for Vector<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Bitvector<{N}>[")?;
        let len = self.len();
        let mut bits_written = 0;
        for (index, bit) in self.iter().enumerate() {
            let value = i32::from(*bit);
            write!(f, "{value}")?;
            bits_written += 1;
            // SAFETY: checked subtraction is unnecessary, as len >= 1 for bitvectors; qed
            if bits_written % 4 == 0 && index != len - 1 {
                write!(f, "_")?;
            }
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl<const N: usize> Default for Vector<N> {
    fn default() -> Self {
        // SAFETY: there is currently no way to enforce statically
        // that `N` is non-zero with const generics so panics are possible.
        assert!(N > 0);

        Self(BitVec::repeat(false, N))
    }
}

#[derive(Debug, Clone, derive_more::Display)]
pub enum Error {
    #[display(fmt = "invalid byte {_0:x} when decoding data of the expected type")]
    InvalidByte(u8),
    #[display(
        fmt = "{} byte(s) given but only expected (up to) {} byte(s)",
        provided,
        expected
    )]
    AdditionalInput { provided: usize, expected: usize },
    #[display(
        fmt = "expected at least {} byte(s) when decoding but provided only {} byte(s)",
        expected,
        provided
    )]
    ExpectedFurtherInput { provided: usize, expected: usize },
    #[display(
        fmt = "{} elements given for a type with (inclusive) upper bound {}",
        provided,
        bound
    )]
    Bounded { bound: usize, provided: usize },
}

impl<const N: usize> Vector<N> {
    /// Return the bit at `index`. `None` if index is out-of-bounds.
    pub fn get(&mut self, index: usize) -> Option<bool> {
        self.0.get(index).map(|value| *value)
    }

    /// Set the bit at `index` to `value`. Return the previous value
    /// or `None` if index is out-of-bounds.
    pub fn set(&mut self, index: usize, value: bool) -> Option<bool> {
        self.0.get_mut(index).map(|mut slot| {
            let old = *slot;
            *slot = value;
            old
        })
    }

    /// Returns a serialized representation.
    ///
    /// The output is faithful to the SSZ encoding of `self`.
    pub fn as_bytes(&self) -> Vec<u8> {
        self.chunks(BITS_PER_BYTE).map(|byte| byte.load()).collect()
    }

    /// Instantiates a new instance from `bytes`. Consumes the same format that `self.into_bytes()`
    /// produces (SSZ).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let expected_length = bytes_for_bit_len(N);
        if bytes.len() < expected_length {
            return Err(Error::ExpectedFurtherInput {
                provided: bytes.len(),
                expected: expected_length,
            });
        }

        if bytes.len() > expected_length {
            return Err(Error::AdditionalInput {
                provided: bytes.len(),
                expected: expected_length,
            });
        }

        let mut result = Self::default();
        for (slot, byte) in result
            .0
            .chunks_mut(BITS_PER_BYTE)
            .zip(bytes.iter().copied())
        {
            slot.store_le(byte);
        }

        let remainder_count = N % BITS_PER_BYTE;
        if remainder_count != 0 {
            let last_byte = bytes.last().unwrap();
            let remainder_bits = last_byte >> remainder_count;
            if remainder_bits != 0 {
                return Err(Error::InvalidByte(*last_byte));
            }
        }

        Ok(result)
    }
}

impl<const N: usize> Deref for Vector<N> {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> TreeHash for Vector<N> {
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
        utils::bitfield_bytes_tree_hash_root::<N>(&self.as_bytes())
    }
}

impl<const N: usize> Decode for Vector<N> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        let bytes = <Vec<u8> as Decode>::decode(input)?;

        Ok(Self(BitVec::try_from_vec(bytes).map_err(|_| {
            parity_scale_codec::Error::from(
                "Failed to construct Vector<N>: source Vec<u8> is too long",
            )
        })?))
    }
}

impl<const N: usize> Encode for Vector<N> {
    fn encode(&self) -> Vec<u8> {
        self.as_raw_slice().encode()
    }

    fn encode_to<W: parity_scale_codec::Output + ?Sized>(&self, dest: &mut W) {
        self.as_raw_slice().encode_to(dest)
    }

    fn encoded_size(&self) -> usize {
        self.as_raw_slice().encoded_size()
    }

    fn size_hint(&self) -> usize {
        self.as_raw_slice().size_hint()
    }

    fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        self.as_raw_slice().using_encoded(f)
    }
}

// +1 for length bit
fn bitlist_byte_length(bound: usize) -> usize {
    (bound + BITS_PER_BYTE - 1 + 1) / BITS_PER_BYTE
}

/// A homogenous collection of a variable number of boolean values.
#[derive(PartialEq, Eq, Clone)]
pub struct List<const N: usize>(Inner);

impl<'de, const N: usize> Deserialize<'de> for List<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = utils::decode_hex_bytes(deserializer)?;

        Self::try_from(&bytes[..]).map_err(<D::Error as de::Error>::custom)
    }
}

impl<const N: usize> fmt::Debug for List<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Bitlist<len={}, cap={N}>[", self.len())?;
        let len = self.len();
        let mut bits_written = 0;
        for (index, bit) in self.iter().enumerate() {
            let value = i32::from(*bit);
            write!(f, "{value}")?;
            bits_written += 1;
            // SAFETY: checked subtraction is unnecessary, as len >= 1 when this for loop runs; qed
            if bits_written % 4 == 0 && index != len - 1 {
                write!(f, "_")?;
            }
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl<const N: usize> Default for List<N> {
    fn default() -> Self {
        Self(BitVec::new())
    }
}

impl<const N: usize> List<N> {
    /// Return the bit at `index`. `None` if index is out-of-bounds.
    pub fn get(&mut self, index: usize) -> Option<bool> {
        self.0.get(index).map(|value| *value)
    }

    /// Set the bit at `index` to `value`. Return the previous value
    /// or `None` if index is out-of-bounds.
    pub fn set(&mut self, index: usize, value: bool) -> Option<bool> {
        self.0.get_mut(index).map(|mut slot| {
            let old = *slot;
            *slot = value;
            old
        })
    }

    fn serialize_with_length(&self, buffer: &mut Vec<u8>, with_length_bit: bool) -> usize {
        let start_len = buffer.len();
        buffer.extend_from_slice(self.as_raw_slice());

        if with_length_bit {
            let element_count = self.len();
            let marker_index = element_count % BITS_PER_BYTE;
            if marker_index == 0 {
                buffer.push(1u8);
            } else {
                let last = buffer.last_mut().expect("bitlist cannot be empty");
                *last |= 1u8 << marker_index;
            }
        }

        // SAFETY: checked subtraction is unnecessary, as buffer.len() > start_len; qed
        buffer.len() - start_len
    }
}

impl<const N: usize> Deref for List<N> {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> TryFrom<&[u8]> for List<N> {
    type Error = Error;

    fn try_from(encoding: &[u8]) -> Result<Self, Self::Error> {
        // validate byte length - min
        let Some((last_byte, prefix)) = encoding.split_last() else {
            return Err(Error::ExpectedFurtherInput {
                provided: 0,
                expected: 1,
            });
        };

        // validate byte length - max
        let max_len = bitlist_byte_length(N);
        if encoding.len() > max_len {
            return Err(Error::AdditionalInput {
                provided: encoding.len(),
                expected: max_len,
            });
        }

        if *last_byte == 0u8 {
            return Err(Error::InvalidByte(*last_byte));
        }

        let mut result = Inner::from_slice(prefix);
        let last = Inner::from_element(*last_byte);

        // validate bit length satisfies bound `N`
        // SAFETY: checked subtraction is unnecessary,
        // as last_byte != 0, so last.trailing_zeros <= 7; qed
        // therefore: bit_length >= 1
        let bit_length = BITS_PER_BYTE - last.trailing_zeros();
        // skip marker bit
        let additional_members = bit_length - 1;
        let total_members = result.len() + additional_members;
        if total_members > N {
            return Err(Error::Bounded {
                bound: N,
                provided: total_members,
            });
        }

        result.extend_from_bitslice(&last[..additional_members]);

        Ok(Self(result))
    }
}

impl<const N: usize> TryFrom<&[bool]> for List<N> {
    type Error = Error;

    fn try_from(value: &[bool]) -> Result<Self, Self::Error> {
        if value.len() > N {
            let len = value.len();
            Err(Error::Bounded {
                bound: N,
                provided: len,
            })
        } else {
            let mut result = Self::default();
            for bit in value {
                result.0.push(*bit);
            }
            Ok(result)
        }
    }
}

impl<const N: usize> TreeHash for List<N> {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        tree_hash::TreeHashType::List
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("List should never be packed.")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("List should never be packed.")
    }

    fn tree_hash_root(&self) -> Hash256 {
        let root = utils::bitfield_bytes_tree_hash_root::<N>(self.as_raw_slice());
        tree_hash::mix_in_length(&root, self.len())
    }
}

impl<const N: usize> Decode for List<N> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        let bytes = <Vec<u8> as Decode>::decode(input)?;

        Self::try_from(&bytes[..]).map_err(|_| {
            parity_scale_codec::Error::from(
                "Failed to construct Vector<N>: source Vec<u8> is too long",
            )
        })
    }
}

impl<const N: usize> Encode for List<N> {
    fn encode(&self) -> Vec<u8> {
        let mut buffer = vec![];
        self.serialize_with_length(&mut buffer, true);

        buffer.encode()
    }

    fn encode_to<W: parity_scale_codec::Output + ?Sized>(&self, dest: &mut W) {
        let mut buffer = vec![];
        self.serialize_with_length(&mut buffer, true);

        buffer.encode_to(dest)
    }

    fn encoded_size(&self) -> usize {
        let mut buffer = vec![];
        self.serialize_with_length(&mut buffer, true);

        buffer.encoded_size()
    }

    fn size_hint(&self) -> usize {
        let mut buffer = vec![];
        self.serialize_with_length(&mut buffer, true);

        buffer.size_hint()
    }

    fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        let mut buffer = vec![];
        self.serialize_with_length(&mut buffer, true);

        buffer.using_encoded(f)
    }
}
