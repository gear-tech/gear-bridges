use super::{
    base_types::List,
    beacon::{BlockHeader, Bytes32, SignedBeaconBlockHeader, SyncAggregate, SyncCommittee},
    memory_db,
    patricia_trie::{TrieDB, TrieDBMut},
    trie_db::{Recorder, Trie, TrieMut},
    Debug, Hash256, TreeHash, TreeHashType, EPOCHS_PER_SYNC_COMMITTEE, H256, SLOTS_PER_EPOCH, U256,
};
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::Log;
use alloy_rlp::Encodable;
use core::{fmt, str::FromStr};
use serde::{de, Deserialize};

const CAPACITY_RLP_RECEIPT: usize = 10_000;

pub type ReceiptEnvelope = alloy_consensus::ReceiptEnvelope<Log>;
/// Tuple with a transaction index and the related receipt.
pub type Receipt = (u64, ReceiptEnvelope);

/// Decode the exact raw EIP-2718 receipt trie value, never its P2P RLP wrapper.
/// Retained originals must not be reencoded to migrate their receipt identity.
pub fn decode_canonical_receipt(bytes: &[u8]) -> alloy_rlp::Result<ReceiptEnvelope> {
    let mut input = bytes;
    let receipt = ReceiptEnvelope::decode_2718(&mut input)?;
    if !input.is_empty() {
        return Err(alloy_rlp::Error::UnexpectedLength);
    }
    if receipt.encoded_2718() != bytes {
        return Err(alloy_rlp::Error::Custom("Noncanonical receipt"));
    }
    Ok(receipt)
}

/// Decode an unchanged historical P2P-framed receipt only after its manager and
/// endpoint CodeIds/range have been independently approved and authenticated.
/// Fresh admission must use `decode_canonical_receipt` instead.
pub fn decode_canonical_network_receipt(bytes: &[u8]) -> alloy_rlp::Result<ReceiptEnvelope> {
    let mut input = bytes;
    let receipt = ReceiptEnvelope::network_decode(&mut input)?;
    if !input.is_empty() {
        return Err(alloy_rlp::Error::UnexpectedLength);
    }
    let mut canonical = Vec::with_capacity(receipt.network_len());
    receipt.network_encode(&mut canonical);
    if canonical != bytes {
        return Err(alloy_rlp::Error::Custom("Noncanonical historical receipt"));
    }
    Ok(receipt)
}

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
#[derive(Deserialize, Debug, Clone)]
pub struct Beacon {
    pub beacon: BlockHeader,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug, Clone)]
pub struct BeaconBlockHeaderResponse {
    pub data: BeaconBlockHeaderData,
}

/// According to Beacon API spec [v2.5.0](https://ethereum.github.io/beacon-APIs/?urls.primaryName=v2.5.0).
#[derive(Deserialize, Debug, Clone)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct GenesisResponse {
    pub data: GenesisData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenesisData {
    #[serde(deserialize_with = "deserialize_u64")]
    pub genesis_time: u64,
    #[serde(deserialize_with = "decode_hex_bytes")]
    pub genesis_validators_root: Vec<u8>,
    #[serde(deserialize_with = "decode_hex_bytes")]
    pub genesis_fork_version: Vec<u8>,
}

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
            status: receipt.as_receipt().expect("supported receipt type").status,
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

pub fn deserialize_list_u64<'de, D, const N: usize>(
    deserializer: D,
) -> Result<List<u64, N>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let list: Vec<&str> = Deserialize::deserialize(deserializer)?;
    let list = list
        .into_iter()
        .map(|value| u64::from_str(value).map_err(de::Error::custom))
        .collect::<Result<Vec<_>, _>>()?;

    List::<_, N>::try_from(list).map_err(de::Error::custom)
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    use alloy_consensus::{Eip658Value, Receipt as ConsensusReceipt, ReceiptWithBloom, TxType};

    // VB-20 reconstructed from HTML; audited revision 7fe1b032.
    // Synthetic vectors test framing, not historical deployed endpoint qualification.
    #[test]
    fn canonical_receipt_types_reject_wrappers_trailing_and_noncanonical_rlp() {
        let body = ReceiptWithBloom::new(
            ConsensusReceipt::<Log> {
                status: true.into(),
                cumulative_gas_used: 21_000,
                logs: Vec::new(),
            },
            Default::default(),
        );
        let mut legacy = hex::decode("f9010801825208b90100").unwrap();
        legacy.extend_from_slice(&[0; 256]);
        legacy.push(0xc0);
        for (tag, ty) in [
            TxType::Legacy,
            TxType::Eip2930,
            TxType::Eip1559,
            TxType::Eip4844,
            TxType::Eip7702,
        ]
        .into_iter()
        .enumerate()
        {
            let mut raw = Vec::new();
            if tag != 0 {
                raw.push(tag as u8);
            }
            raw.extend_from_slice(&legacy);
            let expected = ReceiptEnvelope::from_typed(ty, body.clone());
            assert_eq!(decode_canonical_receipt(&raw).unwrap(), expected);
            assert_eq!(expected.encoded_2718(), raw);

            let mut wrapped = Vec::new();
            raw.as_slice().encode(&mut wrapped);
            assert!(decode_canonical_receipt(&wrapped).is_err());
            let mut trailing = raw.clone();
            trailing.push(0);
            assert!(decode_canonical_receipt(&trailing).is_err());
            assert!(decode_canonical_receipt(&raw[..raw.len() - 1]).is_err());

            // Overlong encoding of status 1, with a matching outer list length.
            let offset = usize::from(tag != 0);
            let mut noncanonical = raw;
            noncanonical[offset + 2] += 1;
            noncanonical.insert(offset + 3, 0x81);
            assert!(decode_canonical_receipt(&noncanonical).is_err());
        }
        for tag in [0, 5, 0x7f] {
            let mut raw = vec![tag];
            raw.extend_from_slice(&legacy);
            assert!(decode_canonical_receipt(&raw).is_err());
        }
        assert!(decode_canonical_receipt(&[]).is_err());
        // Canonical decoding does not itself assert economic success.
        let mut failed = body;
        failed.receipt.status = false.into();
        assert_eq!(
            decode_canonical_receipt(&ReceiptEnvelope::Legacy(failed).encoded_2718())
                .unwrap()
                .as_receipt()
                .unwrap()
                .status,
            Eip658Value::Eip658(false)
        );
    }

    // VB-20 / Phase 3E.6 reconstructed from HTML; audited revision 7fe1b032.
    #[cfg(feature = "std")]
    #[test]
    fn rpc_post_state_mapping_preserves_exact_receipt_and_trie_root() {
        use crate::{hash_db::Hasher, keccak_hasher::KeccakHasher};

        let status = Eip658Value::PostState(alloy_primitives::B256::repeat_byte(0x42));
        let rpc = alloy_consensus::ReceiptEnvelope::Legacy(ReceiptWithBloom::new(
            ConsensusReceipt::<alloy::rpc::types::Log> {
                status,
                cumulative_gas_used: 21_000,
                logs: Vec::new(),
            },
            Default::default(),
        ));
        let mapped = map_receipt_envelope(&rpc);
        let mut raw = hex::decode("f90128a0").unwrap();
        raw.extend_from_slice(&[0x42; 32]);
        raw.extend_from_slice(&hex::decode("825208b90100").unwrap());
        raw.extend_from_slice(&[0; 256]);
        raw.push(0xc0);
        assert_eq!(mapped.as_receipt().unwrap().status, status);
        assert_eq!(mapped.encoded_2718(), raw);
        assert_eq!(decode_canonical_receipt(&raw).unwrap(), mapped);

        // Independently encode the single-leaf MPT node for transaction index 0.
        let mut leaf = rlp::RlpStream::new_list(2);
        leaf.append(&[0x20u8, 0x80].as_slice());
        leaf.append(&raw.as_slice());
        let expected_root = KeccakHasher::hash(&leaf.out());
        let root_for = |receipt: &ReceiptEnvelope| {
            let mut db = memory_db::new();
            let mut root = H256::zero();
            let mut trie = TrieDBMut::new(&mut db, &mut root);
            let (key, value) = rlp_encode_index_and_receipt(&0, receipt);
            trie.insert(&key, &value).unwrap();
            *trie.root()
        };
        assert_eq!(root_for(&mapped), expected_root);
        let mut coerced = mapped;
        coerced.as_receipt_with_bloom_mut().unwrap().receipt.status = rpc.status().into();
        assert_ne!(root_for(&coerced), expected_root);
    }
}
