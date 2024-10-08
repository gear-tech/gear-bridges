use super::{
    patricia_trie::{TrieDB, TrieDBMut},
    trie_db::{Recorder, Trie, TrieMut},
    *,
};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Log;
use alloy_rlp::Encodable;
use core::str::FromStr;

const CAPACITY_RLP_RECEIPT: usize = 10_000;

pub type ReceiptEnvelope = alloy_consensus::ReceiptEnvelope<Log>;
/// Tuple with a transaction index and the related receipt.
pub type Receipt = (u64, ReceiptEnvelope);

#[derive(Clone, Debug)]
pub enum ProofError {
    ReceiptNotFound,
    InsertionFailed,
    RootIsNotValid,
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

impl core::error::Error for ProofError {}

pub struct Proof {
    pub proof: Vec<Vec<u8>>,
    pub receipt: ReceiptEnvelope,
}

pub fn calculate_epoch(slot: u64) -> u64 {
    slot / SLOTS_PER_EPOCH
}

pub fn calculate_period(slot: u64) -> u64 {
    calculate_epoch(slot) / EPOCHS_PER_SYNC_COMMITTEE
}

pub fn calculate_slot(period: u64) -> u64 {
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

/// A helper function providing common functionality for finding the Merkle root of some bytes that
/// represent a bitfield.
pub fn bitfield_bytes_tree_hash_root<const N: usize>(bytes: &[u8]) -> Hash256 {
    use tree_hash::{MerkleHasher, BYTES_PER_CHUNK};

    let byte_size = (N + 7) / 8;
    let leaf_count = (byte_size + BYTES_PER_CHUNK - 1) / BYTES_PER_CHUNK;

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
    use alloy_consensus::{Receipt, ReceiptWithBloom, TxType};

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
    }
}

pub fn generate_proof(tx_index: u64, receipts: &[Receipt]) -> Result<Proof, ProofError> {
    let mut memory_db = memory_db::new();
    let key_value_tuples = rlp_encode_receipts_and_nibble_tuples(receipts);
    let root = {
        let mut root = H256::zero();
        let mut triedbmut = TrieDBMut::new(&mut memory_db, &mut root);
        for (key, value) in &key_value_tuples {
            triedbmut
                .insert(key, value)
                .map_err(|_| ProofError::InsertionFailed)?;
        }

        *triedbmut.root()
    };

    let (tx_index, receipt) = receipts
        .iter()
        .find(|(index, _)| index == &tx_index)
        .ok_or(ProofError::ReceiptNotFound)?;

    let trie = TrieDB::new(&memory_db, &root).map_err(|_| ProofError::RootIsNotValid)?;
    let (key, _expected_value) = rlp_encode_index_and_receipt(tx_index, receipt);

    let mut recorder = Recorder::new();
    let _value = trie.get_with(&key, &mut recorder);

    Ok(Proof {
        proof: recorder.drain().into_iter().map(|r| r.data).collect(),
        receipt: receipt.clone(),
    })
}
