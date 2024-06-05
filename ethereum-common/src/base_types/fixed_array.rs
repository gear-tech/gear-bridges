// This file is part of Gear.

// Copyright (C) 2024 Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use super::*;

#[derive(Clone)]
pub struct FixedArray<T, const N: usize>(pub [T; N]);

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
