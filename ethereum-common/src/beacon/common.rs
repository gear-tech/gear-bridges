//! The module implement types used defined by Ethereum Beacon Chain spec v1.4.0.
//!
//! Inspired by <https://github.com/a16z/helios> and <https://github.com/sigp/lighthouse> projects.

use super::*;

pub type Bytes32 = base_types::BytesFixed<32>;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/bellatrix/beacon-chain.md#custom-types).
pub type Address = base_types::BytesFixed<20>;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/bellatrix/beacon-chain.md#execution).
pub type LogsBloom = base_types::BytesFixed<256>;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/bellatrix/beacon-chain.md#custom-types).
pub type Transaction = base_types::ByteList<1_073_741_824>;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#custom-types).
pub type SignatureBytes = base_types::BytesFixed<96>;
/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#custom-types).
pub type BLSPubKey = base_types::BytesFixed<48>;

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/capella/beacon-chain.md#withdrawal).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct Withdrawal {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub index: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
    pub address: Address,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub amount: u64,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#eth1data).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct Eth1Data {
    pub deposit_root: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub deposit_count: u64,
    pub block_hash: Bytes32,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#signedbeaconblockheader).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct SignedBeaconBlockHeader {
    pub message: BlockHeader,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#proposerslashing).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct ProposerSlashing {
    pub signed_header_1: SignedBeaconBlockHeader,
    pub signed_header_2: SignedBeaconBlockHeader,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#attesterslashing).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct AttesterSlashing {
    pub attestation_1: IndexedAttestation,
    pub attestation_2: IndexedAttestation,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#indexedattestation).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct IndexedAttestation {
    pub attesting_indices: base_types::List<u64, 2_048>,
    pub data: AttestationData,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#attestation).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct Attestation {
    pub aggregation_bits: base_types::Bitlist<2_048>,
    pub data: AttestationData,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#attestationdata).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct AttestationData {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub slot: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub index: u64,
    pub beacon_block_root: Bytes32,
    pub source: Checkpoint,
    pub target: Checkpoint,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#checkpoint).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct Checkpoint {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub epoch: u64,
    pub root: Bytes32,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#deposit).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct Deposit {
    pub proof: base_types::FixedArray<Bytes32, 33>,
    pub data: DepositData,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#depositdata).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct DepositData {
    pub pubkey: BLSPubKey,
    pub withdrawal_credentials: Bytes32,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub amount: u64,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#signedvoluntaryexit).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct SignedVoluntaryExit {
    pub message: VoluntaryExit,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#voluntaryexit).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct VoluntaryExit {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub epoch: u64,
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/capella/beacon-chain.md#signedblstoexecutionchange).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct SignedBlsToExecutionChange {
    pub message: BlsToExecutionChange,
    pub signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/capella/beacon-chain.md#blstoexecutionchange).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct BlsToExecutionChange {
    #[serde(deserialize_with = "utils::deserialize_u64")]
    pub validator_index: u64,
    pub from_bls_pubkey: BLSPubKey,
    pub to_execution_address: Address,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/beacon-chain.md#syncaggregate).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct SyncAggregate {
    pub sync_committee_bits: base_types::Bitvector<512>,
    pub sync_committee_signature: SignatureBytes,
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/altair/beacon-chain.md#synccommittee).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
pub struct SyncCommittee {
    pub pubkeys: base_types::FixedArray<BLSPubKey, SYNC_COMMITTEE_SIZE>,
    pub aggregate_pubkey: BLSPubKey,
}

pub mod electra {
    use super::*;
    use crate::electra::*;

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#depositrequest).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct DepositRequest {
        pubkey: BLSPubKey,
        withdrawal_credentials: Bytes32,
        amount: u64,
        signature: SignatureBytes,
        index: u64,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#withdrawalrequest).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct WithdrawalRequest {
        source_address: Address,
        validator_pubkey: BLSPubKey,
        amount: u64,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#consolidationrequest).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct ConsolidationRequest {
        source_address: Address,
        source_pubkey: BLSPubKey,
        target_pubkey: BLSPubKey,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#executionrequests).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct ExecutionRequests {
        deposits: base_types::List<DepositRequest, { MAX_DEPOSIT_REQUESTS_PER_PAYLOAD as usize }>,
        withdrawals:
            base_types::List<WithdrawalRequest, { MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD as usize }>,
        consolidations: base_types::List<
            ConsolidationRequest,
            { MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD as usize },
        >,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#indexedattestation).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct IndexedAttestation {
        pub attesting_indices: base_types::List<
            u64,
            { (MAX_VALIDATORS_PER_COMMITTEE * MAX_COMMITTEES_PER_SLOT) as usize },
        >,
        pub data: AttestationData,
        pub signature: SignatureBytes,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#attestation).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct Attestation {
        pub aggregation_bits: base_types::Bitlist<
            { (MAX_VALIDATORS_PER_COMMITTEE * MAX_COMMITTEES_PER_SLOT) as usize },
        >,
        pub data: AttestationData,
        pub signature: SignatureBytes,
        pub committee_bits: base_types::Bitvector<{ MAX_COMMITTEES_PER_SLOT as usize }>,
    }

    /// According to Ethereum spec [v1.5.0](https://github.com/ethereum/consensus-specs/blob/v1.5.0-beta.2/specs/electra/beacon-chain.md#attesterslashing).
    #[derive(
        Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
    )]
    pub struct AttesterSlashing {
        pub attestation_1: IndexedAttestation,
        pub attestation_2: IndexedAttestation,
    }
}
