use super::*;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct SyncCommitteeUpdate {
    pub signature_slot: u64,
    pub attested_header: BeaconBlockHeader,
    pub finalized_header: BeaconBlockHeader,
    pub sync_aggregate: SyncAggregate,
    pub sync_committee_signature: ArkScale<G2TypeInfo>,
    pub sync_committee_next_aggregate_pubkey: Option<BLSPubKey>,
    pub sync_committee_next_pub_keys: Option<Box<SyncCommitteeKeys>>,
    pub sync_committee_next_branch: Option<Vec<[u8; 32]>>,
    pub finality_branch: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Error {
    InvalidTimestamp,
    InvalidPeriod,
    LowVoteCount,
    NotActual,
    InvalidSignature,
    InvalidFinalityProof,
    InvalidNextSyncCommitteeProof,
    InvalidPublicKeys,
}
