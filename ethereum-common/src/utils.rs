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

pub fn decode_hex_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let bytes: &str = Deserialize::deserialize(deserializer)?;
    let bytes = match bytes.starts_with("0x") {
        true => &bytes[2..],
        false => bytes,
    };

    hex::decode(bytes).map_err(<D::Error as de::Error>::custom)
}

/// A helper function providing common functionality between the `TreeHash` implementations for
/// the fixed array and variable list types.
pub fn vec_tree_hash_root<T, const N: usize>(vec: &[T]) -> Hash256
where
    T: TreeHash,
{
    use tree_hash::MerkleHasher;

    match T::tree_hash_type() {
        TreeHashType::Basic => {
            let mut hasher = MerkleHasher::with_leaves(
                (N + T::tree_hash_packing_factor() - 1) / T::tree_hash_packing_factor(),
            );

            for item in vec {
                hasher
                    .write(&item.tree_hash_packed_encoding())
                    .expect("ssz_types variable vec should not contain more elements than max");
            }

            hasher
                .finish()
                .expect("ssz_types variable vec should not have a remaining buffer")
        }

        TreeHashType::Container | TreeHashType::List | TreeHashType::Vector => {
            let mut hasher = MerkleHasher::with_leaves(N);

            for item in vec {
                hasher
                    .write(item.tree_hash_root().as_bytes())
                    .expect("ssz_types vec should not contain more elements than max");
            }

            hasher
                .finish()
                .expect("ssz_types vec should not have a remaining buffer")
        }
    }
}
