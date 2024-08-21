use super::*;
use alloy_consensus::ReceiptEnvelope;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Log;
use alloy_rlp::Encodable;
use core::str::FromStr;

const CAPACITY_RLP_RECEIPT: usize = 10_000;

/// Tuple with a transaction index and the related receipt.
pub type Receipt = (u64, ReceiptEnvelope<Log>);

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

pub fn rlp_encode_index_and_receipt(
    index: &u64,
    receipt: &ReceiptEnvelope<Log>,
) -> (Vec<u8>, Vec<u8>) {
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
