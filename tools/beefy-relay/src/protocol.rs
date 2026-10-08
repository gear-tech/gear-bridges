use anyhow::{ensure, Context, Result};
use binary_merkle_tree::{merkle_proof, merkle_root, verify_proof};
use hash_db::Hasher;
use k256::{ecdsa::Signature as K256Signature, elliptic_curve::sec1::ToEncodedPoint};
use parity_scale_codec::{Compact, Decode, Encode};
use sp_consensus_beefy::{
    ecdsa_crypto::{AuthorityId, Signature},
    mmr::BeefyAuthoritySet,
    SignedCommitment, ValidatorSet, VersionedFinalityProof,
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

pub const SNAPSHOT_VERSION: u8 = 2;
pub const SNAPSHOT_LEN: usize = 86;
pub const OUTER_LEAF_LEN: usize = 113;
pub const MAX_MMR_PROOF_ITEMS: usize = 256;
pub const MAX_VALIDATORS: usize = 256;
pub const MAX_BEEFY_PROOF_BYTES: usize = MAX_BEEFY_PAYLOAD_BYTES + MAX_VALIDATORS * 65 + 512;
pub const MAX_BEEFY_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_BEEFY_PAYLOAD_ITEMS: usize = 64;
pub const MAX_MMR_LEAVES_BYTES: usize = 1 + 2 + OUTER_LEAF_LEN;
pub const MAX_MMR_PROOF_BYTES: usize = 1 + 8 + 8 + 2 + MAX_MMR_PROOF_ITEMS * 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub bridge_domain: Hash32,
    pub source_timestamp_ms: u64,
    pub initialized: bool,
    pub queue_id: u64,
    pub queue_root: Hash32,
}

impl QueueSnapshot {
    pub fn new(
        bridge_domain: Hash32,
        source_timestamp_ms: u64,
        initialized: bool,
        queue_id: u64,
        queue_root: Hash32,
    ) -> Result<Self> {
        let snapshot = Self {
            bridge_domain,
            source_timestamp_ms,
            initialized,
            queue_id,
            queue_root,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.bridge_domain != [0; 32],
            "bridge domain is unconfigured"
        );
        ensure!(
            self.initialized || (self.queue_id == 0 && self.queue_root == [0; 32]),
            "uninitialized snapshot must have zero queue id and root"
        );
        Ok(())
    }

    pub fn encode(&self) -> [u8; SNAPSHOT_LEN] {
        let mut out = [0; SNAPSHOT_LEN];
        out[0] = SNAPSHOT_VERSION;
        out[1..5].copy_from_slice(b"vara");
        out[5..37].copy_from_slice(&self.bridge_domain);
        out[37..45].copy_from_slice(&self.source_timestamp_ms.to_le_bytes());
        out[45] = u8::from(self.initialized);
        out[46..54].copy_from_slice(&self.queue_id.to_le_bytes());
        out[54..].copy_from_slice(&self.queue_root);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == SNAPSHOT_LEN,
            "snapshot must be {SNAPSHOT_LEN} bytes"
        );
        ensure!(bytes[0] == SNAPSHOT_VERSION, "unsupported snapshot version");
        ensure!(&bytes[1..5] == b"vara", "snapshot magic is not vara");
        ensure!(
            matches!(bytes[45], 0 | 1),
            "snapshot initialized flag is not bool"
        );
        let mut bridge_domain = [0; 32];
        bridge_domain.copy_from_slice(&bytes[5..37]);
        let mut queue_root = [0; 32];
        queue_root.copy_from_slice(&bytes[54..]);
        Self::new(
            bridge_domain,
            u64::from_le_bytes(bytes[37..45].try_into().expect("fixed length")),
            bytes[45] == 1,
            u64::from_le_bytes(bytes[46..54].try_into().expect("fixed length")),
            queue_root,
        )
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
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

pub fn bridge_domain(source_domain: Hash32, chain_id: u64, queue: [u8; 20]) -> Result<Hash32> {
    ensure!(
        source_domain != [0; 32] && chain_id != 0 && queue != [0; 20],
        "unconfigured bridge destination"
    );
    const PREFIX: &[u8] = b"vara/gear-eth-bridge-domain/v2";
    let mut preimage = [0u8; PREFIX.len() + 32 + 32 + 20];
    let mut offset = PREFIX.len();
    preimage[..offset].copy_from_slice(PREFIX);
    preimage[offset..offset + 32].copy_from_slice(&source_domain);
    offset += 32 + 24;
    preimage[offset..offset + 8].copy_from_slice(&chain_id.to_be_bytes());
    offset += 8;
    preimage[offset..].copy_from_slice(&queue);
    Ok(keccak256(&preimage))
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KeccakHasher;

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
    ensure!(
        (1..=MAX_VALIDATORS).contains(&keys.len()),
        "authority count must be between 1 and {MAX_VALIDATORS}"
    );
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
    ensure!(
        (1..=MAX_VALIDATORS).contains(&addresses.len()),
        "authority count must be between 1 and {MAX_VALIDATORS}"
    );
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
    if !(1..=MAX_VALIDATORS).contains(&set_len) || proof.index as usize >= set_len {
        return false;
    }
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

// Scan lengths and fixed-width bodies without allocating. The upstream signed
// commitment decoder expands its compressed bitfield outside codec allocation hooks.
fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    ensure!(len <= input.len(), "truncated SCALE body");
    let (value, rest) = input.split_at(len);
    *input = rest;
    Ok(value)
}

fn vector_len(input: &mut &[u8], max: usize) -> Result<usize> {
    let len = Compact::<u32>::decode(input)?.0 as usize;
    ensure!(len <= max, "SCALE vector exceeds limit {max}");
    Ok(len)
}

fn scan_payload(input: &mut &[u8]) -> Result<()> {
    let start = input.len();
    let count = vector_len(input, MAX_BEEFY_PAYLOAD_ITEMS)?;
    let mut previous = None;
    for _ in 0..count {
        let id: [u8; 2] = take(input, 2)?.try_into().expect("fixed payload id");
        ensure!(
            previous.is_none_or(|previous| previous < id),
            "BEEFY payload ids are not canonical"
        );
        previous = Some(id);
        let len = vector_len(input, MAX_BEEFY_PAYLOAD_BYTES)?;
        take(input, len)?;
        ensure!(
            start - input.len() <= MAX_BEEFY_PAYLOAD_BYTES,
            "BEEFY payload exceeds body limit"
        );
    }
    Ok(())
}

fn scan_authorities(input: &mut &[u8]) -> Result<()> {
    let len = vector_len(input, MAX_VALIDATORS)?;
    ensure!(len != 0, "empty authority set");
    take(input, len * 33)?;
    Ok(())
}

pub fn decode_authority_list(bytes: &[u8]) -> Result<Vec<AuthorityId>> {
    let mut input = bytes;
    scan_authorities(&mut input)?;
    ensure!(input.is_empty(), "authority list has trailing bytes");
    decode_full(bytes, "authority list")
}

pub fn decode_validator_set(bytes: &[u8]) -> Result<Option<ValidatorSet<AuthorityId>>> {
    let mut input = bytes;
    match take(&mut input, 1)?[0] {
        0 => {}
        1 => {
            scan_authorities(&mut input)?;
            take(&mut input, 8)?;
        }
        _ => anyhow::bail!("invalid authority set option"),
    }
    ensure!(input.is_empty(), "authority set has trailing bytes");
    decode_full(bytes, "authority set")
}

pub fn decode_mmr_leaves(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() <= MAX_MMR_LEAVES_BYTES,
        "MMR leaves exceed body limit"
    );
    let mut input = bytes;
    ensure!(
        vector_len(&mut input, 1)? == 1,
        "MMR proof must contain one source leaf"
    );
    ensure!(
        vector_len(&mut input, OUTER_LEAF_LEN)? == OUTER_LEAF_LEN,
        "invalid MMR leaf width"
    );
    let leaf = take(&mut input, OUTER_LEAF_LEN)?;
    ensure!(input.is_empty(), "MMR leaves have trailing bytes");
    decode_outer_leaf(leaf)?;
    // Compact<u32> rejects nonminimal prefixes; the fixed leaf decoder checks its encoding.
    Ok(leaf.to_vec())
}

pub fn decode_mmr_proof(bytes: &[u8]) -> Result<RuntimeLeafProof> {
    ensure!(
        bytes.len() <= MAX_MMR_PROOF_BYTES,
        "MMR proof exceeds body limit"
    );
    let mut input = bytes;
    ensure!(
        vector_len(&mut input, 1)? == 1,
        "MMR proof must have one leaf index"
    );
    take(&mut input, 16)?; // One index and leaf count.
    let len = vector_len(&mut input, MAX_MMR_PROOF_ITEMS)?;
    take(&mut input, len * 32)?;
    ensure!(input.is_empty(), "MMR proof has trailing bytes");
    decode_full(bytes, "MMR proof")
}

pub fn decode_beefy_digest(bytes: &[u8]) -> Result<Option<Hash32>> {
    use sp_consensus_beefy::ConsensusLog;
    use sp_runtime::generic::DigestItem;
    ensure!(
        bytes.len() <= MAX_VALIDATORS * 33 + 32,
        "digest exceeds body limit"
    );
    let mut input = bytes;
    match take(&mut input, 1)?[0] {
        0 => {
            let len = vector_len(&mut input, MAX_VALIDATORS * 33 + 16)?;
            take(&mut input, len)?;
        }
        4 | 5 | 6 => {
            take(&mut input, 4)?;
            let len = vector_len(&mut input, MAX_VALIDATORS * 33 + 16)?;
            take(&mut input, len)?;
        }
        8 => {}
        _ => anyhow::bail!("unknown digest variant"),
    }
    ensure!(input.is_empty(), "digest has trailing bytes");
    let item: DigestItem = decode_full(bytes, "digest")?;
    let DigestItem::Consensus(engine, payload) = item else {
        return Ok(None);
    };
    if engine != *b"BEEF" {
        return Ok(None);
    }
    let mut input = payload.as_slice();
    match take(&mut input, 1)?[0] {
        1 => {
            scan_authorities(&mut input)?;
            take(&mut input, 8)?;
        }
        2 => {
            take(&mut input, 4)?;
        }
        3 => {
            take(&mut input, 32)?;
        }
        _ => anyhow::bail!("unknown BEEFY consensus variant"),
    }
    ensure!(
        input.is_empty(),
        "BEEFY consensus digest has trailing bytes"
    );
    let consensus: ConsensusLog<AuthorityId> = decode_full(&payload, "BEEFY consensus digest")?;
    match consensus {
        ConsensusLog::MmrRoot(root) => Ok(Some(root.into())),
        ConsensusLog::AuthoritiesChange(set) => {
            let keys: Vec<[u8; 33]> = set
                .validators()
                .iter()
                .map(|key| {
                    <AuthorityId as AsRef<[u8]>>::as_ref(key)
                        .try_into()
                        .expect("fixed authority width")
                })
                .collect();
            authority_addresses(&keys)?;
            Ok(None)
        }
        ConsensusLog::OnDisabled(_) => Ok(None),
    }
}

pub fn decode_versioned_finality_proof(bytes: &[u8]) -> Result<RuntimeSignedCommitment> {
    ensure!(
        bytes.len() <= MAX_BEEFY_PROOF_BYTES,
        "BEEFY proof exceeds body limit"
    );
    let mut input = bytes;
    ensure!(
        take(&mut input, 1)?[0] == 1,
        "unsupported BEEFY proof version"
    );
    scan_payload(&mut input)?;
    take(&mut input, 12)?; // Block number and authority set id.
    let bitfield_len = vector_len(&mut input, MAX_VALIDATORS / 8 + 1)?;
    let bitfield = take(&mut input, bitfield_len)?;
    let set_len =
        u32::from_le_bytes(take(&mut input, 4)?.try_into().expect("fixed width")) as usize;
    ensure!(
        (1..=MAX_VALIDATORS).contains(&set_len),
        "invalid signature authority count"
    );
    // Upstream SCALE pack always appends a full zero byte when N is divisible by 8.
    ensure!(
        bitfield_len == set_len / 8 + 1,
        "invalid signature bitfield width"
    );
    let padding = 8 - set_len % 8;
    ensure!(
        bitfield[bitfield_len - 1] & ((1u16 << padding) - 1) as u8 == 0,
        "nonzero signature padding"
    );
    let count = vector_len(&mut input, MAX_VALIDATORS)?;
    ensure!(
        count
            == bitfield
                .iter()
                .map(|byte| byte.count_ones() as usize)
                .sum::<usize>(),
        "signature count differs from bitfield"
    );
    take(&mut input, count * 65)?;
    ensure!(input.is_empty(), "BEEFY proof has trailing bytes");
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

pub fn validate_signed_commitment(
    bytes: &[u8],
    authority_keys: &[[u8; 33]],
    expected_set_id: Option<u64>,
    expected_block: Option<u32>,
) -> Result<ValidatedCommitment> {
    ensure!(
        (1..=MAX_VALIDATORS).contains(&authority_keys.len()),
        "authority count must be between 1 and {MAX_VALIDATORS}"
    );
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

    let root = signed
        .commitment
        .payload
        .get_raw(b"mh")
        .context("BEEFY commitment must contain exactly one mh payload")?;
    ensure!(root.len() == 32, "mh payload must be 32 bytes");
    let mmr_root: Hash32 = root.as_slice().try_into().expect("checked length");
    ensure!(mmr_root != [0; 32], "mh payload root must be nonzero");

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
    if proof.leaf_count == 1 {
        ensure!(
            index == 0 && proof.items.is_empty(),
            "single-leaf MMR proof is not canonical"
        );
        return Ok(root == leaf_hash);
    }
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
    snapshot: &QueueSnapshot,
    anchor_block: u64,
    anchor_root: Hash32,
    leaf: &RuntimeLeaf,
    items: &[Hash32],
    proof_order: [u8; 32],
) -> Result<Vec<u8>> {
    snapshot.validate()?;
    ensure!(
        items.len() <= MAX_MMR_PROOF_ITEMS,
        "MMR proof path is too long"
    );
    if items.len() < MAX_MMR_PROOF_ITEMS {
        let used = highest_used_order_bit(&proof_order)
            .map(|bit| bit + 1)
            .unwrap_or(0);
        ensure!(used <= items.len(), "MMR proof order has unused bits");
    }
    let raw_leaf = encode_outer_leaf(leaf)?;
    let mut out = Vec::with_capacity(576 + 32 * items.len());
    out.extend_from_slice(&abi_word(u64::from(SNAPSHOT_VERSION)));
    out.extend_from_slice(&abi_word(u64::from(SNAPSHOT_VERSION)));
    out.extend_from_slice(&abi_word(u64::from(snapshot.initialized)));
    out.extend_from_slice(&snapshot.bridge_domain);
    out.extend_from_slice(&abi_word(snapshot.source_timestamp_ms));
    out.extend_from_slice(&abi_word(snapshot.queue_id));
    out.extend_from_slice(&abi_word(anchor_block));
    out.extend_from_slice(&anchor_root);
    out.extend_from_slice(&abi_word(raw_leaf[0] as u64));
    out.extend_from_slice(&abi_word_u32(leaf.parent_number_and_hash.0));
    out.extend_from_slice(&leaf.parent_number_and_hash.1);
    out.extend_from_slice(&abi_word(leaf.beefy_next_authority_set.id));
    out.extend_from_slice(&abi_word_u32(leaf.beefy_next_authority_set.len));
    out.extend_from_slice(&leaf.beefy_next_authority_set.keyset_commitment);
    out.extend_from_slice(&leaf.leaf_extra);
    out.extend_from_slice(&abi_word(17 * 32));
    out.extend_from_slice(&proof_order);
    out.extend_from_slice(&abi_word(items.len() as u64));
    for item in items {
        out.extend_from_slice(item);
    }
    ensure!(
        out.len() == 576 + 32 * items.len(),
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

    fn scalar_key(value: u16) -> [u8; 33] {
        let mut scalar = [0; 32];
        scalar[30..].copy_from_slice(&value.to_be_bytes());
        k256::ecdsa::SigningKey::from_bytes((&scalar).into())
            .expect("valid scalar")
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .expect("compressed key")
    }

    #[test]
    fn snapshot_matches_gear_runtime_fixture() {
        let domain = bridge_domain([0x22; 32], 31_337, [0x33; 20]).unwrap();
        assert_eq!(
            hex::encode(domain),
            "9aac6d72e183672d20696112082accb15719870152120212f541e3e233837944"
        );
        assert_ne!(
            domain,
            bridge_domain([0x22; 32], 31_338, [0x33; 20]).unwrap()
        );
        assert_ne!(
            domain,
            bridge_domain([0x22; 32], 31_337, [0x34; 20]).unwrap()
        );
        assert_ne!(
            domain,
            bridge_domain([0x23; 32], 31_337, [0x33; 20]).unwrap()
        );
        assert!(bridge_domain([0; 32], 31_337, [0x33; 20]).is_err());
        assert!(bridge_domain([0x22; 32], 0, [0x33; 20]).is_err());
        assert!(bridge_domain([0x22; 32], 31_337, [0; 20]).is_err());
        let snapshot = QueueSnapshot::new(
            domain,
            1_800_000_000_000,
            true,
            0x0102_0304_0506_0708,
            [0x11; 32],
        )
        .expect("canonical snapshot");
        assert_eq!(
            hex::encode(snapshot.encode()),
            "02766172619aac6d72e183672d20696112082accb15719870152120212f541e3e23383794400505c18a30100000108070605040302011111111111111111111111111111111111111111111111111111111111111111"
        );
        assert_eq!(
            hex::encode(snapshot.hash()),
            "bd5de17e8ba7e515ecc4a3cf986ea31348d459f03cd620c9b7b64da34bc49228"
        );
        let uninitialized = QueueSnapshot::new(domain, 1_800_000_000_000, false, 0, [0; 32])
            .expect("canonical uninitialized snapshot");
        assert_eq!(
            hex::encode(uninitialized.hash()),
            "b288ac0954894c55094a5e4031ff98669bf8d16e4ed43068908b6053e0e19444"
        );
        let initialized_empty = QueueSnapshot::new(domain, 1_800_000_000_000, true, 0, [0; 32])
            .expect("canonical initialized-empty snapshot");
        assert_eq!(
            hex::encode(initialized_empty.hash()),
            "74f1bc7cab4ffb32c5607968f6a11bda72a8b685406df623fb23fd8f17153b5c"
        );
        assert!(QueueSnapshot::new([0; 32], 0, false, 1, [0; 32]).is_err());
        assert_eq!(QueueSnapshot::decode(&snapshot.encode()).unwrap(), snapshot);
        let mut legacy = snapshot.encode();
        legacy[0] = 1;
        assert!(QueueSnapshot::decode(&legacy).is_err());
        legacy = snapshot.encode();
        legacy[5..37].fill(0);
        assert!(QueueSnapshot::decode(&legacy).is_err());
        legacy = snapshot.encode();
        legacy[1] = b'V';
        assert!(QueueSnapshot::decode(&legacy).is_err());
        legacy = snapshot.encode();
        legacy[45] = 2;
        assert!(QueueSnapshot::decode(&legacy).is_err());
        legacy = uninitialized.encode();
        legacy[46] = 1;
        assert!(QueueSnapshot::decode(&legacy).is_err());
        assert!(QueueSnapshot::decode(&snapshot.encode()[..85]).is_err());
    }

    #[test]
    fn authority_tree_preserves_positions_and_bounds() {
        let keys: Vec<[u8; 33]> = (1..=3).map(scalar_key).collect();
        let addresses = authority_addresses(&keys).expect("valid keys");
        let root = authority_root(&addresses).expect("root");
        for index in 0..keys.len() {
            let proof = authority_proof(&keys, index).expect("proof");
            assert!(verify_authority_proof(&root, &proof, keys.len()));
        }
        assert!(authority_addresses(&[]).is_err());
        assert!(!verify_authority_proof(
            &root,
            &AuthorityProof {
                index: 3,
                address: addresses[0],
                siblings: Vec::new(),
                root,
            },
            keys.len()
        ));
    }

    #[test]
    fn queue_envelope_uses_v2_geometry() {
        let snapshot = QueueSnapshot::new([0x22; 32], 1_800_000_000_000, true, 7, [0x11; 32])
            .expect("snapshot");
        let leaf = make_leaf(4, [0x33; 32], 2, 3, [0x44; 32], snapshot.hash());
        let encoded = encode_queue_proof(&snapshot, 8, [0x55; 32], &leaf, &[], [0; 32])
            .expect("queue envelope");
        assert_eq!(encoded.len(), 576);
        assert_eq!(&encoded[0..32], &abi_word(2));
        assert_eq!(&encoded[32..64], &abi_word(2));
        assert_eq!(&encoded[480..512], &abi_word(17 * 32));
        assert_eq!(&encoded[544..576], &abi_word(0));
        assert!(encode_queue_proof(&snapshot, 8, [0x55; 32], &leaf, &[], [1; 32]).is_err());
    }
}
