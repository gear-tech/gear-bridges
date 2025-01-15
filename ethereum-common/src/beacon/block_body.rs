use super::*;

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/deneb/beacon-chain.md#beaconblockbody).
#[derive(
    Debug, Clone, Decode, Encode, Deserialize, PartialEq, tree_hash_derive::TreeHash, TypeInfo,
)]
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
