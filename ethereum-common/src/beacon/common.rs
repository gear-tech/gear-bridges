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

//! The module defines types used by Beacon Chain entities. Inspired by
//! https://github.com/a16z/helios and https://github.com/sigp/lighthouse/ projects.

use super::*;

pub type Bytes32 = base_types::BytesFixed<32>;
pub type Address = base_types::BytesFixed<20>;
pub type LogsBloom = base_types::BytesFixed<256>;
pub type Transaction = base_types::ByteList<1_073_741_824>;
pub type SignatureBytes = base_types::BytesFixed<96>;
pub type BLSPubKey = base_types::BytesFixed<48>;

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct Withdrawal {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub index: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
    pub address: Address,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub amount: u64,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct Eth1Data {
    pub deposit_root: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub deposit_count: u64,
    pub block_hash: Bytes32,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct SignedBeaconBlockHeader {
    pub message: BlockHeader,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct ProposerSlashing {
    pub signed_header_1: SignedBeaconBlockHeader,
    pub signed_header_2: SignedBeaconBlockHeader,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct AttesterSlashing {
    pub attestation_1: IndexedAttestation,
    pub attestation_2: IndexedAttestation,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct IndexedAttestation {
    pub attesting_indices: base_types::List<u64, 2_048>,
    pub data: AttestationData,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct Attestation {
    pub aggregation_bits: base_types::Bitlist<2_048>,
    pub data: AttestationData,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct AttestationData {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub slot: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub index: u64,
    pub beacon_block_root: Bytes32,
    pub source: Checkpoint,
    pub target: Checkpoint,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct Checkpoint {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub epoch: u64,
    pub root: Bytes32,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct Deposit {
    pub proof: base_types::FixedArray<Bytes32, 33>,
    pub data: DepositData,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct DepositData {
    pub pubkey: BLSPubKey,
    pub withdrawal_credentials: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub amount: u64,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct SignedVoluntaryExit {
    pub message: VoluntaryExit,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct VoluntaryExit {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub epoch: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct SignedBlsToExecutionChange {
    pub message: BlsToExecutionChange,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct BlsToExecutionChange {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
    pub from_bls_pubkey: BLSPubKey,
    pub to_execution_address: Address,
}

#[derive(Debug, Clone, Decode, Encode, Deserialize, tree_hash_derive::TreeHash)]
pub struct SyncAggregate {
    pub sync_committee_bits: base_types::Bitvector<512>,
    pub sync_committee_signature: SignatureBytes,
}
