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

#[derive(Debug, Clone, Decode, Encode, tree_hash_derive::TreeHash, Deserialize)]
pub struct BlockBody {
    pub randao_reveal: SignatureBytes,
    pub eth1_data: Eth1Data,
    pub graffiti: Bytes32,
    pub proposer_slashings: base_types::List<ProposerSlashing, 16>,
    pub attester_slashings: base_types::List<AttesterSlashing, 2>,
    pub attestations: base_types::List<Attestation, 128>,
    pub deposits: base_types::List<Deposit, 16>,
    pub voluntary_exits: base_types::List<SignedVoluntaryExit, 16>,
    pub sync_aggregate: SyncAggregate,
    pub execution_payload: ExecutionPayload,
    pub bls_to_execution_changes: base_types::List<SignedBlsToExecutionChange, 16>,
    pub blob_kzg_commitments: base_types::List<base_types::BytesFixed<48>, 4_096>,
}
