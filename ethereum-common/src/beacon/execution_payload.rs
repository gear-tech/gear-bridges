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

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct ExecutionPayload {
    pub parent_hash: Bytes32,
    pub fee_recipient: Address,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub block_number: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub gas_limit: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub gas_used: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub timestamp: u64,
    pub extra_data: base_types::ByteList<32>,
    #[serde(deserialize_with = "utils::deserialize_u256")]
    pub base_fee_per_gas: U256,
    pub block_hash: Bytes32,
    pub transactions: base_types::List<Transaction, 1_048_576>,
    pub withdrawals: base_types::List<Withdrawal, 16>,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub blob_gas_used: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub excess_blob_gas: u64,
}
