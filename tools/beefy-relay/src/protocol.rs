use anyhow::{ensure, Context, Result};
use binary_merkle_tree::{merkle_proof, merkle_root, verify_proof};
use hash_db::Hasher;
use k256::{ecdsa::Signature as K256Signature, elliptic_curve::sec1::ToEncodedPoint};
use parity_scale_codec::{Decode, Encode};
use sp_consensus_beefy::{
    ecdsa_crypto::{AuthorityId, Signature},
    mmr::BeefyAuthoritySet,
    Commitment, SignedCommitment, ValidatorSet, VersionedFinalityProof,
};
use sp_core::crypto::UncheckedFrom;
use sp_mmr_primitives::{mmr_lib, LeafProof};
use sp_runtime::traits::Keccak256;
use std::collections::BTreeSet;
use tiny_keccak::{Hasher as _, Keccak};

pub type Hash32 = [u8; 32];
pub type RuntimeLeaf = sp_consensus_beefy::mmr::MmrLeaf<u32, Hash32, Hash32, Hash32>;
pub type RuntimeLeafProof = LeafProof<Hash32>;
pub type RuntimeSignedCommitment = SignedCommitment<u32, Signature>;

pub const SNAPSHOT_VERSION_INITIALIZED: u8 = 0;
pub const SNAPSHOT_VERSION_UNINITIALIZED: u8 = u8::MAX;
pub const SNAPSHOT_LEN: usize = 45;
pub const OUTER_LEAF_LEN: usize = 113;
pub const MAX_MMR_PROOF_ITEMS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub version: u8,
    pub queue_id: u64,
    pub queue_root: Hash32,
}

impl QueueSnapshot {
    pub fn new(version: u8, queue_id: u64, queue_root: Hash32) -> Self {
        Self {
            version,
            queue_id,
            queue_root,
        }
    }

    pub fn encode(&self) -> [u8; SNAPSHOT_LEN] {
        let mut out = [0; SNAPSHOT_LEN];
        out[0] = self.version;
        out[1..5].copy_from_slice(b"vara");
        out[5..13].copy_from_slice(&self.queue_id.to_le_bytes());
        out[13..].copy_from_slice(&self.queue_root);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == SNAPSHOT_LEN,
            "snapshot must be {SNAPSHOT_LEN} bytes"
        );
        ensure!(&bytes[1..5] == b"vara", "snapshot magic is not vara");
        let mut root = [0; 32];
        root.copy_from_slice(&bytes[13..]);
        Ok(Self {
            version: bytes[0],
            queue_id: u64::from_le_bytes(bytes[5..13].try_into().expect("fixed length")),
            queue_root: root,
        })
    }

    pub fn is_initialized(&self) -> bool {
        self.version == SNAPSHOT_VERSION_INITIALIZED
    }

    pub fn hash(&self) -> Hash32 {
        keccak256(&self.encode())
    }
}

pub fn keccak256(bytes: &[u8]) -> Hash32 {
    let mut out = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut out);
    out
}

#[derive(Clone, Copy, Debug, Default)]
struct KeccakHasher;

impl Hasher for KeccakHasher {
    type Out = Hash32;
    type StdHasher = std::collections::hash_map::DefaultHasher;
    const LENGTH: usize = 32;

    fn hash(x: &[u8]) -> Self::Out {
        keccak256(x)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityProof {
    pub index: u32,
    pub address: [u8; 20],
    pub siblings: Vec<Hash32>,
    pub root: Hash32,
}

pub fn authority_address(compressed_key: &[u8]) -> Result<[u8; 20]> {
    ensure!(
        compressed_key.len() == 33,
        "ECDSA authority key must be 33 bytes"
    );
    ensure!(
        matches!(compressed_key[0], 2 | 3),
        "authority key is not compressed SEC1"
    );
    let key = k256::PublicKey::from_sec1_bytes(compressed_key)
        .map_err(|_| anyhow::anyhow!("invalid compressed authority key"))?;
    let encoded = key.to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Ok(digest[12..].try_into().expect("fixed address length"))
}

pub fn authority_addresses(keys: &[[u8; 33]]) -> Result<Vec<[u8; 20]>> {
    let mut seen_keys = BTreeSet::new();
    let mut seen_addresses = BTreeSet::new();
    keys.iter()
        .map(|key| {
            let address = authority_address(key)?;
            ensure!(seen_keys.insert(*key), "duplicate authority key");
            ensure!(
                seen_addresses.insert(address),
                "duplicate authority address"
            );
            Ok(address)
        })
        .collect()
}

pub fn authority_root(addresses: &[[u8; 20]]) -> Result<Hash32> {
    ensure!(!addresses.is_empty(), "authority set cannot be empty");
    let mut seen = BTreeSet::new();
    for address in addresses {
        ensure!(seen.insert(*address), "duplicate authority address");
    }
    Ok(merkle_root::<KeccakHasher, _>(addresses.iter()))
}

pub fn authority_proof(keys: &[[u8; 33]], index: usize) -> Result<AuthorityProof> {
    let addresses = authority_addresses(keys)?;
    ensure!(index < addresses.len(), "authority index outside set");
    let proof = merkle_proof::<KeccakHasher, _, _>(addresses.iter(), index);
    Ok(AuthorityProof {
        index: index as u32,
        address: addresses[index],
        siblings: proof.proof,
        root: proof.root,
    })
}

pub fn verify_authority_proof(root: &Hash32, proof: &AuthorityProof, set_len: usize) -> bool {
    verify_proof::<KeccakHasher, _, _>(
        root,
        proof.siblings.iter().copied(),
        set_len,
        proof.index as usize,
        &proof.address,
    )
}

pub fn make_leaf(
    parent_number: u32,
    parent_hash: Hash32,
    next_authority_set_id: u64,
    next_authority_set_len: u32,
    next_authority_set_root: Hash32,
    bridge_extra: Hash32,
) -> RuntimeLeaf {
    RuntimeLeaf {
        version: Default::default(),
        parent_number_and_hash: (parent_number, parent_hash),
        beefy_next_authority_set: BeefyAuthoritySet {
            id: next_authority_set_id,
            len: next_authority_set_len,
            keyset_commitment: next_authority_set_root,
        },
        leaf_extra: bridge_extra,
    }
}

pub fn encode_outer_leaf(leaf: &RuntimeLeaf) -> Result<Vec<u8>> {
    let encoded = leaf.encode();
    ensure!(
        encoded.len() == OUTER_LEAF_LEN,
        "unexpected MMR leaf size: {}",
        encoded.len()
    );
    ensure!(
        leaf.version.split() == (0, 0),
        "unsupported MMR leaf version"
    );
    Ok(encoded)
}

pub fn decode_outer_leaf(bytes: &[u8]) -> Result<RuntimeLeaf> {
    ensure!(
        bytes.len() == OUTER_LEAF_LEN,
        "outer leaf must be {OUTER_LEAF_LEN} bytes"
    );
    let mut input = bytes;
    let leaf = RuntimeLeaf::decode(&mut input).context("decode MMR leaf")?;
    ensure!(input.is_empty(), "outer leaf has trailing bytes");
    ensure!(
        leaf.version.split() == (0, 0),
        "unsupported MMR leaf version"
    );
    ensure!(leaf.encode() == bytes, "outer leaf is not canonical SCALE");
    Ok(leaf)
}

pub fn outer_leaf_hash(leaf: &RuntimeLeaf) -> Result<Hash32> {
    Ok(keccak256(&encode_outer_leaf(leaf)?))
}

fn decode_full<T: Decode + Encode>(bytes: &[u8], what: &str) -> Result<T> {
    let mut input = bytes;
    let decoded = T::decode(&mut input).with_context(|| format!("decode {what}"))?;
    ensure!(input.is_empty(), "{what} has trailing bytes");
    ensure!(decoded.encode() == bytes, "{what} is not canonical SCALE");
    Ok(decoded)
}

pub fn decode_versioned_finality_proof(bytes: &[u8]) -> Result<RuntimeSignedCommitment> {
    let proof: VersionedFinalityProof<u32, Signature> =
        decode_full(bytes, "versioned BEEFY proof")?;
    match proof {
        VersionedFinalityProof::V1(signed) => Ok(signed),
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedCommitment {
    pub signed: RuntimeSignedCommitment,
    pub commitment_bytes: Vec<u8>,
    pub commitment_hash: Hash32,
    pub mmr_root: Hash32,
    pub signed_indices: Vec<u32>,
}

fn payload_entries(commitment: &Commitment<u32>) -> Result<Vec<([u8; 2], Vec<u8>)>> {
    let bytes = commitment.payload.encode();
    decode_full(&bytes, "BEEFY payload")
}

pub fn validate_signed_commitment(
    bytes: &[u8],
    authority_keys: &[[u8; 33]],
    expected_set_id: Option<u64>,
    expected_block: Option<u32>,
) -> Result<ValidatedCommitment> {
    ensure!(!authority_keys.is_empty(), "authority set cannot be empty");
    authority_addresses(authority_keys)?;
    let signed = decode_versioned_finality_proof(bytes)?;
    if let Some(set_id) = expected_set_id {
        ensure!(
            signed.commitment.validator_set_id == set_id,
            "unexpected authority set id"
        );
    }
    if let Some(block) = expected_block {
        ensure!(
            signed.commitment.block_number == block,
            "unexpected commitment block"
        );
    }

    let entries = payload_entries(&signed.commitment)?;
    ensure!(
        entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "BEEFY payload ids are not canonical"
    );
    let root = entries
        .iter()
        .find(|(id, _)| id == b"mh")
        .context("BEEFY commitment must contain exactly one mh payload")?;
    ensure!(root.1.len() == 32, "mh payload must be 32 bytes");
    let mmr_root: Hash32 = root.1.as_slice().try_into().expect("checked length");

    let authority_ids: Vec<_> = authority_keys
        .iter()
        .map(|key| AuthorityId::unchecked_from(*key))
        .collect();
    let validator_set = ValidatorSet::new(authority_ids, signed.commitment.validator_set_id)
        .ok_or_else(|| anyhow::anyhow!("authority set cannot be empty"))?;
    ensure!(
        signed.signatures.len() == validator_set.len(),
        "signature vector does not match authority set"
    );
    for signature in signed.signatures.iter().flatten() {
        let raw: &[u8] = signature.as_ref();
        ensure!(raw[64] <= 1, "unsupported ECDSA recovery id");
        let parsed = K256Signature::from_slice(&raw[..64]).context("invalid ECDSA signature")?;
        ensure!(
            parsed.normalize_s().is_none(),
            "high-S ECDSA signature rejected"
        );
    }
    let valid = signed
        .verify_signatures::<AuthorityId, Keccak256>(signed.commitment.block_number, &validator_set)
        .map_err(|_| anyhow::anyhow!("BEEFY signature layout does not match authority set"))?;
    ensure!(
        valid.len() == signed.signature_count(),
        "one or more available BEEFY signatures is invalid"
    );
    ensure!(
        valid.len() >= authority_keys.len() - (authority_keys.len() - 1) / 3,
        "insufficient BEEFY signatures"
    );
    let signed_indices = signed
        .signatures
        .iter()
        .enumerate()
        .filter_map(|(index, signature)| signature.as_ref().map(|_| index as u32))
        .collect();
    let commitment_bytes = signed.commitment.encode();
    Ok(ValidatedCommitment {
        signed,
        commitment_hash: keccak256(&commitment_bytes),
        commitment_bytes,
        mmr_root,
        signed_indices,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimplifiedMmrProof {
    pub items: Vec<Hash32>,
    pub proof_order: [u8; 32],
}

struct KeccakMerge;

impl mmr_lib::Merge for KeccakMerge {
    type Item = Hash32;

    fn merge(left: &Self::Item, right: &Self::Item) -> mmr_lib::Result<Self::Item> {
        let mut bytes = [0; 64];
        bytes[..32].copy_from_slice(left);
        bytes[32..].copy_from_slice(right);
        Ok(keccak256(&bytes))
    }
}

fn mmr_size(leaf_count: u64) -> Result<u64> {
    ensure!(leaf_count > 0, "MMR leaf count must be nonzero");
    let peaks = leaf_count.count_ones() as u64;
    leaf_count
        .checked_mul(2)
        .and_then(|value| value.checked_sub(peaks))
        .ok_or_else(|| anyhow::anyhow!("MMR size overflow"))
}

fn set_order_bit(order: &mut [u8; 32], index: usize, value: bool) -> Result<()> {
    ensure!(index < 256, "MMR proof order exceeds uint256");
    if value {
        order[31 - index / 8] |= 1 << (index % 8);
    }
    Ok(())
}

fn order_bit(order: &[u8; 32], index: usize) -> bool {
    (order[31 - index / 8] & (1 << (index % 8))) != 0
}

fn highest_used_order_bit(order: &[u8; 32]) -> Option<usize> {
    order.iter().enumerate().find_map(|(byte, value)| {
        if *value == 0 {
            None
        } else {
            Some((31 - byte) * 8 + (7 - value.leading_zeros() as usize))
        }
    })
}

fn path_order(
    target_position: u64,
    target_peak: u64,
    target_height: u8,
    path_len: usize,
) -> Result<[u8; 32]> {
    ensure!(
        path_len == target_height as usize,
        "MMR path length does not match target peak"
    );
    let mut order = [0; 32];
    let mut position = target_position;
    let mut height = 0u8;
    for item in 0..path_len {
        let next_height = mmr_lib::helper::pos_height_in_tree(
            position
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("MMR position overflow"))?,
        );
        let parent_offset = 2u64
            .checked_shl(height as u32)
            .ok_or_else(|| anyhow::anyhow!("MMR parent offset overflow"))?;
        if next_height > height {
            set_order_bit(&mut order, item, true)?;
            position = position
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("MMR parent overflow"))?;
        } else {
            position = position
                .checked_add(parent_offset)
                .ok_or_else(|| anyhow::anyhow!("MMR parent overflow"))?;
        }
        height = height
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("MMR height overflow"))?;
    }
    ensure!(
        position == target_peak,
        "MMR proof path does not terminate at peak"
    );
    Ok(order)
}

pub fn leaf_index_to_position(index: u64) -> Result<u64> {
    mmr_size(index.checked_add(1).context("MMR leaf index overflow")?)?;
    Ok(mmr_lib::leaf_index_to_pos(index))
}

pub fn validate_mmr_coordinates(
    proof: &RuntimeLeafProof,
    start: u64,
    source: u32,
    anchor: u64,
) -> Result<()> {
    let insertion = u64::from(source)
        .checked_add(1)
        .context("insertion block overflow")?;
    ensure!(
        insertion >= start && anchor >= insertion,
        "invalid MMR source/anchor range"
    );
    let index = insertion
        .checked_sub(start)
        .context("MMR index underflow")?;
    let count = anchor
        .checked_sub(start)
        .and_then(|v| v.checked_add(1))
        .context("MMR count overflow")?;
    ensure!(
        proof.leaf_indices == [index] && proof.leaf_count == count,
        "MMR coordinates disagree with source and anchor"
    );
    Ok(())
}

pub fn verify_native_mmr_proof(
    root: Hash32,
    proof: &RuntimeLeafProof,
    leaf_hash: Hash32,
) -> Result<bool> {
    ensure!(
        proof.leaf_indices.len() == 1,
        "MMR proof must contain exactly one leaf"
    );
    let index = proof.leaf_indices[0];
    ensure!(
        index < proof.leaf_count,
        "MMR leaf index outside leaf count"
    );
    ensure!(
        proof.items.len() <= MAX_MMR_PROOF_ITEMS,
        "MMR proof path is too long"
    );
    let size = mmr_size(proof.leaf_count)?;
    let position = leaf_index_to_position(index)?;
    // Native verification alone can accept a missing right peak; require exact proof consumption.
    let simplified = convert_mmr_proof(proof)?;
    let native = mmr_lib::MerkleProof::<Hash32, KeccakMerge>::new(size, proof.items.clone());
    let valid = native
        .verify(root, vec![(position, leaf_hash)])
        .map_err(|error| anyhow::anyhow!("native MMR proof verification failed: {error:?}"))?;
    Ok(valid && verify_simplified_mmr(root, leaf_hash, &simplified)?)
}

pub fn convert_mmr_proof(proof: &RuntimeLeafProof) -> Result<SimplifiedMmrProof> {
    ensure!(
        proof.leaf_indices.len() == 1,
        "MMR proof must contain exactly one leaf"
    );
    let index = proof.leaf_indices[0];
    ensure!(
        index < proof.leaf_count,
        "MMR leaf index outside leaf count"
    );
    ensure!(
        proof.items.len() <= MAX_MMR_PROOF_ITEMS,
        "MMR proof path is too long"
    );
    let peaks = mmr_lib::helper::get_peaks(mmr_size(proof.leaf_count)?);
    let position = leaf_index_to_position(index)?;
    let target = peaks
        .iter()
        .position(|peak| position <= *peak)
        .context("MMR leaf outside peaks")?;
    let peak = peaks[target];
    let height = mmr_lib::helper::pos_height_in_tree(peak);
    let has_right_peak = target + 1 < peaks.len();
    let path_end = target + usize::from(height);
    let expected = path_end + usize::from(has_right_peak);
    ensure!(
        proof.items.len() == expected,
        "MMR proof item count does not match peaks and path"
    );
    let base = if target == 0 {
        0
    } else {
        peaks[target - 1].checked_add(1).context("peak overflow")?
    };
    let mut order = path_order(position - base, peak - base, height, usize::from(height))?;
    let mut items = Vec::with_capacity(expected);
    items.extend_from_slice(&proof.items[target..path_end]);
    if has_right_peak {
        set_order_bit(&mut order, items.len(), true)?;
        items.push(proof.items[path_end]);
    }
    items.extend(proof.items[..target].iter().rev().copied());
    Ok(SimplifiedMmrProof {
        items,
        proof_order: order,
    })
}

pub fn verify_simplified_mmr(
    root: Hash32,
    leaf_hash: Hash32,
    proof: &SimplifiedMmrProof,
) -> Result<bool> {
    ensure!(
        proof.items.len() <= MAX_MMR_PROOF_ITEMS,
        "MMR proof path is too long"
    );
    if proof.items.len() < MAX_MMR_PROOF_ITEMS {
        let used = highest_used_order_bit(&proof.proof_order)
            .map(|bit| bit + 1)
            .unwrap_or(0);
        ensure!(used <= proof.items.len(), "MMR proof order has unused bits");
    }
    let mut acc = leaf_hash;
    for (index, sibling) in proof.items.iter().enumerate() {
        let mut bytes = [0; 64];
        if order_bit(&proof.proof_order, index) {
            bytes[..32].copy_from_slice(sibling);
            bytes[32..].copy_from_slice(&acc);
        } else {
            bytes[..32].copy_from_slice(&acc);
            bytes[32..].copy_from_slice(sibling);
        }
        acc = keccak256(&bytes);
    }
    Ok(acc == root)
}

fn abi_word(value: u64) -> Hash32 {
    let mut word = [0; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn abi_word_u32(value: u32) -> Hash32 {
    let mut word = [0; 32];
    word[28..].copy_from_slice(&value.to_be_bytes());
    word
}

pub fn encode_queue_proof(
    proof_version: u8,
    bridge_version: u8,
    queue_id: u64,
    anchor_block: u64,
    anchor_root: Hash32,
    leaf: &RuntimeLeaf,
    items: &[Hash32],
    proof_order: [u8; 32],
) -> Result<Vec<u8>> {
    ensure!(proof_version == 0, "unsupported queue proof version");
    ensure!(bridge_version == 0, "unsupported bridge version");
    ensure!(
        items.len() <= MAX_MMR_PROOF_ITEMS,
        "MMR proof path is too long"
    );
    let raw_leaf = encode_outer_leaf(leaf)?;
    let mut head = Vec::with_capacity(14 * 32);
    head.extend_from_slice(&abi_word(proof_version as u64));
    head.extend_from_slice(&abi_word(bridge_version as u64));
    head.extend_from_slice(&abi_word(queue_id));
    head.extend_from_slice(&abi_word(anchor_block));
    head.extend_from_slice(&anchor_root);
    head.extend_from_slice(&abi_word(raw_leaf[0] as u64));
    head.extend_from_slice(&abi_word_u32(leaf.parent_number_and_hash.0));
    head.extend_from_slice(&leaf.parent_number_and_hash.1);
    head.extend_from_slice(&abi_word(leaf.beefy_next_authority_set.id));
    head.extend_from_slice(&abi_word_u32(leaf.beefy_next_authority_set.len));
    head.extend_from_slice(&leaf.beefy_next_authority_set.keyset_commitment);
    head.extend_from_slice(&leaf.leaf_extra);
    head.extend_from_slice(&abi_word(14 * 32));
    head.extend_from_slice(&proof_order);
    let mut out = head;
    out.extend_from_slice(&abi_word(items.len() as u64));
    for item in items {
        out.extend_from_slice(item);
    }
    ensure!(
        out.len() == 480 + 32 * items.len(),
        "queue proof ABI length mismatch"
    );
    Ok(out)
}

pub fn encode_public_inputs(queue_root: Hash32, source_block: u32) -> Result<[Hash32; 2]> {
    let mut p0 = [0; 32];
    p0[8..].copy_from_slice(&queue_root[..24]);
    let mut p1 = [0; 32];
    p1[8..16].copy_from_slice(&queue_root[24..]);
    p1[16..20].copy_from_slice(&source_block.to_be_bytes());
    Ok([p0, p1])
}

pub fn public_inputs_bytes(inputs: &[Hash32; 2]) -> [u8; 64] {
    let mut out = [0; 64];
    out[..32].copy_from_slice(&inputs[0]);
    out[32..].copy_from_slice(&inputs[1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_matches_gear_runtime_fixture() {
        let root = [0x11; 32];
        let snapshot = QueueSnapshot::new(0, 0x0102_0304_0506_0708, root);
        assert_eq!(hex::encode(snapshot.encode()), "007661726108070605040302011111111111111111111111111111111111111111111111111111111111111111");
        assert_eq!(
            hex::encode(snapshot.hash()),
            "5181adf4e1620049e3aeeca5ddad0fca137f2456f336c38341c645a810569980"
        );
        let uninitialized = QueueSnapshot::new(255, 0, [0; 32]);
        assert_eq!(
            hex::encode(uninitialized.hash()),
            "c2fe11d2d2b8e3dc8102033a2dad52ad4a61e6e15e6037273f9132a85293c2de"
        );
    }

    #[test]
    fn authority_tree_preserves_positions() {
        let keys: Vec<[u8; 33]> = (1u8..=3)
            .map(|value| {
                let signing_key =
                    k256::ecdsa::SigningKey::from_bytes((&[value; 32]).into()).expect("valid key");
                signing_key
                    .verifying_key()
                    .to_encoded_point(true)
                    .as_bytes()
                    .try_into()
                    .expect("compressed key")
            })
            .collect();
        let addresses = authority_addresses(&keys).expect("valid keys");
        let root = authority_root(&addresses).expect("root");
        for index in 0..keys.len() {
            let proof = authority_proof(&keys, index).expect("proof");
            assert!(verify_authority_proof(&root, &proof, keys.len()));
        }
    }
}
