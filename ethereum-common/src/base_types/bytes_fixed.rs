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

#[derive(Clone, Debug, Decode, Encode)]
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
