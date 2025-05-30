use super::{
    beacon::{BlockHeader, Bytes32, SignedBeaconBlockHeader, SyncAggregate, SyncCommittee},
    memory_db,
    patricia_trie::{TrieDB, TrieDBMut},
    trie_db::{Recorder, Trie, TrieMut},
    Debug, Hash256, TreeHash, TreeHashType, EPOCHS_PER_SYNC_COMMITTEE, H256, SLOTS_PER_EPOCH, U256,
};
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Log;
use alloy_rlp::Encodable;
use core::{fmt, str::FromStr};
use serde::{de, Deserialize};

const CAPACITY_RLP_RECEIPT: usize = 10_000;

pub type ReceiptEnvelope = alloy_consensus::ReceiptEnvelope<Log>;
/// Tuple with a transaction index and the related receipt.
pub type Receipt = (u64, ReceiptEnvelope);

/// Describes possible errors in generating Merkle proof.
#[derive(Clone, Debug)]
pub enum MerkleProofError {
    /// Receipt with the specified transaction index was not found.
    ReceiptNotFound,
    /// Failed to insert a pair into TrieDB.
    InsertionFailed,
    /// Root of the TrieDB is not valid.
    RootIsNotValid,
}

impl fmt::Display for MerkleProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

impl core::error::Error for MerkleProofError {}

/// Contains Merkle proof for the provided receipt.
pub struct MerkleProof {
    pub proof: Vec<Vec<u8>>,
    pub receipt: ReceiptEnvelope,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum LightClientHeader {
    Unwrapped(BlockHeader),
    Wrapped(Beacon),
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
pub struct Beacon {
    pub beacon: BlockHeader,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
pub struct BeaconBlockHeaderResponse {
    pub data: BeaconBlockHeaderData,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
pub struct BeaconBlockHeaderData {
    pub header: SignedBeaconBlockHeader,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
pub struct BeaconBlockResponse<Block> {
    pub data: BeaconBlockData<Block>,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug)]
pub struct BeaconBlockData<Block> {
    pub message: Block,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Bootstrap {
    #[serde(deserialize_with = "deserialize_block_header")]
    pub header: BlockHeader,
    pub current_sync_committee: SyncCommittee,
    pub current_sync_committee_branch: Vec<Bytes32>,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct BootstrapResponse {
    pub data: Bootstrap,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize)]
pub struct FinalityUpdateResponse {
    pub data: FinalityUpdate,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Clone, Deserialize)]
pub struct FinalityUpdate {
    #[serde(deserialize_with = "deserialize_block_header")]
    pub attested_header: BlockHeader,
    #[serde(deserialize_with = "deserialize_block_header")]
    pub finalized_header: BlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "deserialize_u64")]
    pub signature_slot: u64,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    #[serde(deserialize_with = "deserialize_block_header")]
    pub attested_header: BlockHeader,
    pub next_sync_committee: SyncCommittee,
    pub next_sync_committee_branch: Vec<Bytes32>,
    #[serde(deserialize_with = "deserialize_block_header")]
    pub finalized_header: BlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "deserialize_u64")]
    pub signature_slot: u64,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateData {
    pub data: Update,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
pub type UpdateResponse = Vec<UpdateData>;

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_epoch_at_slot).
pub const fn calculate_epoch(slot: u64) -> u64 {
    slot / SLOTS_PER_EPOCH
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_epoch_at_slot).
pub const fn calculate_period(slot: u64) -> u64 {
    calculate_epoch(slot) / EPOCHS_PER_SYNC_COMMITTEE
}

/// According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#compute_start_slot_at_epoch).
pub const fn calculate_slot(period: u64) -> u64 {
    period * SLOTS_PER_EPOCH * EPOCHS_PER_SYNC_COMMITTEE
}

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

pub fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: &str = Deserialize::deserialize(deserializer)?;

    u64::from_str(value).map_err(<D::Error as de::Error>::custom)
}

pub fn deserialize_u256<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val: &str = Deserialize::deserialize(deserializer)?;

    U256::from_dec_str(val).map_err(<D::Error as de::Error>::custom)
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
            let mut hasher = MerkleHasher::with_leaves(N.div_ceil(T::tree_hash_packing_factor()));

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

/// A helper function providing common functionality for finding the Merkle root of some bytes that
/// represent a bitfield.
pub fn bitfield_bytes_tree_hash_root<const N: usize>(bytes: &[u8]) -> Hash256 {
    use tree_hash::{MerkleHasher, BYTES_PER_CHUNK};

    let byte_size = N.div_ceil(8);
    let leaf_count = byte_size.div_ceil(BYTES_PER_CHUNK);

    let mut hasher = MerkleHasher::with_leaves(leaf_count);

    hasher
        .write(bytes)
        .expect("bitfield should not exceed tree hash leaf limit");

    hasher
        .finish()
        .expect("bitfield tree hash buffer should not exceed leaf limit")
}

pub fn rlp_encode_transaction_index(index: &u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(100);
    Encodable::encode(&index, &mut buf);

    buf
}

pub fn rlp_encode_index_and_receipt(index: &u64, receipt: &ReceiptEnvelope) -> (Vec<u8>, Vec<u8>) {
    let mut buf = Vec::with_capacity(CAPACITY_RLP_RECEIPT);
    receipt.encode_2718(&mut buf);

    (rlp_encode_transaction_index(index), buf)
}

pub fn rlp_encode_receipts_and_nibble_tuples(receipts: &[Receipt]) -> Vec<(Vec<u8>, Vec<u8>)> {
    receipts
        .iter()
        .map(|(transaction_index, receipt)| {
            rlp_encode_index_and_receipt(transaction_index, receipt)
        })
        .collect::<Vec<_>>()
}

#[cfg(feature = "std")]
pub fn map_receipt_envelope(
    receipt: &alloy_consensus::ReceiptEnvelope<alloy::rpc::types::Log>,
) -> ReceiptEnvelope {
    use alloy_consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom, TxType};

    let logs = receipt.logs().iter().map(AsRef::as_ref).cloned().collect();

    let result = ReceiptWithBloom::new(
        Receipt {
            status: receipt.status().into(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs,
        },
        *receipt.logs_bloom(),
    );

    match receipt.tx_type() {
        TxType::Legacy => ReceiptEnvelope::Legacy(result),
        TxType::Eip1559 => ReceiptEnvelope::Eip1559(result),
        TxType::Eip2930 => ReceiptEnvelope::Eip2930(result),
        TxType::Eip4844 => ReceiptEnvelope::Eip4844(result),
        TxType::Eip7702 => ReceiptEnvelope::Eip7702(result),
    }
}

/// Generates Merkle proof for the provided transaction index `tx_index`.
///
/// Returns `MerkleProofError` on failure.
pub fn generate_merkle_proof(
    tx_index: u64,
    receipts: &[Receipt],
) -> Result<MerkleProof, MerkleProofError> {
    let mut memory_db = memory_db::new();
    let key_value_tuples = rlp_encode_receipts_and_nibble_tuples(receipts);
    let root = {
        let mut root = H256::zero();
        let mut triedbmut = TrieDBMut::new(&mut memory_db, &mut root);
        for (key, value) in &key_value_tuples {
            triedbmut
                .insert(key, value)
                .map_err(|_| MerkleProofError::InsertionFailed)?;
        }

        *triedbmut.root()
    };

    let (tx_index, receipt) = receipts
        .iter()
        .find(|(index, _)| index == &tx_index)
        .ok_or(MerkleProofError::ReceiptNotFound)?;

    let trie = TrieDB::new(&memory_db, &root).map_err(|_| MerkleProofError::RootIsNotValid)?;
    let (key, _expected_value) = rlp_encode_index_and_receipt(tx_index, receipt);

    let mut recorder = Recorder::new();
    let _value = trie.get_with(&key, &mut recorder);

    Ok(MerkleProof {
        proof: recorder.drain().into_iter().map(|r| r.data).collect(),
        receipt: receipt.clone(),
    })
}

pub fn deserialize_block_header<'de, D>(deserializer: D) -> Result<BlockHeader, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let header: LightClientHeader = Deserialize::deserialize(deserializer)?;

    Ok(match header {
        LightClientHeader::Unwrapped(header) => header,
        LightClientHeader::Wrapped(header) => header.beacon,
    })
}
