use anyhow::{ensure, Result};
use k256::ecdsa::SigningKey;
use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use sp_consensus_beefy::{Commitment, Payload, SignedCommitment, VersionedFinalityProof};
use sp_core::crypto::UncheckedFrom;
use sp_mmr_primitives::mmr_lib;
use std::{collections::BTreeMap, path::Path};

use crate::{
    protocol::{
        authority_addresses, authority_root, convert_mmr_proof, encode_outer_leaf,
        encode_public_inputs, encode_queue_proof, keccak256, public_inputs_bytes,
        verify_native_mmr_proof, verify_simplified_mmr, Hash32, KeccakHasher, QueueSnapshot,
        RuntimeLeafProof, MAX_MMR_PROOF_ITEMS, MAX_VALIDATORS,
    },
    validate_signed_commitment,
};

const GEAR_BASELINE_COMMIT: &str = "0b13f2c61b0e5d9844c7efd12727487a2fdb8c63";
const SNOWBRIDGE_BASELINE_COMMIT: &str = "1201293e482ef052b9c3989dcf680046704fef3d";
const BASE_SOURCE_TIMESTAMP_MS: u64 = 1_800_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteropFixture {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "gearBaselineCommit")]
    pub gear_baseline_commit: String,
    #[serde(rename = "snowbridgeBaselineCommit")]
    pub snowbridge_baseline_commit: String,
    pub cases: Vec<InteropCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadItem {
    #[serde(rename = "payloadID")]
    pub payload_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreshnessProof {
    #[serde(rename = "sourceGenesis")]
    pub source_genesis: String,
    #[serde(rename = "bridgeDomain")]
    pub bridge_domain: String,
    #[serde(rename = "sourceTimestampMs")]
    pub source_timestamp_ms: u64,
    pub initialized: bool,
    #[serde(rename = "queueId")]
    pub queue_id: u64,
    #[serde(rename = "queueRoot")]
    pub queue_root: String,
    #[serde(rename = "snapshotPreimage")]
    pub snapshot_preimage: String,
    #[serde(rename = "bridgeCommitment")]
    pub bridge_commitment: String,
    #[serde(rename = "outerLeaf")]
    pub outer_leaf: String,
    #[serde(rename = "outerLeafHash")]
    pub outer_leaf_hash: String,
    #[serde(rename = "leafIndex")]
    pub leaf_index: u64,
    #[serde(rename = "leafCount")]
    pub leaf_count: u64,
    #[serde(rename = "mmrItems")]
    pub mmr_items: Vec<String>,
    #[serde(rename = "simplifiedItems")]
    pub simplified_items: Vec<String>,
    #[serde(rename = "proofOrder")]
    pub proof_order: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteropCase {
    pub name: String,
    #[serde(rename = "nativeOnly")]
    pub native_only: bool,
    #[serde(rename = "mmrStartBlock")]
    pub mmr_start_block: u64,
    #[serde(rename = "sourceBlock")]
    pub source_block: u32,
    #[serde(rename = "insertionBlock")]
    pub insertion_block: u64,
    #[serde(rename = "anchorBlock")]
    pub anchor_block: u64,
    #[serde(rename = "queueId")]
    pub queue_id: u64,
    #[serde(rename = "bridgeVersion")]
    pub bridge_version: u8,
    #[serde(rename = "sourceGenesis")]
    pub source_genesis: String,
    #[serde(rename = "sourceDomain")]
    pub source_domain: String,
    #[serde(rename = "destinationChainId")]
    pub destination_chain_id: u64,
    #[serde(rename = "destinationQueue")]
    pub destination_queue: String,
    #[serde(rename = "bridgeDomain")]
    pub bridge_domain: String,
    #[serde(rename = "sourceTimestampMs")]
    pub source_timestamp_ms: u64,
    pub initialized: bool,
    #[serde(rename = "queueRoot")]
    pub queue_root: String,
    #[serde(rename = "snapshotPreimage")]
    pub snapshot_preimage: String,
    #[serde(rename = "bridgeCommitment")]
    pub bridge_commitment: String,
    #[serde(rename = "outerLeaf")]
    pub outer_leaf: String,
    #[serde(rename = "outerLeafHash")]
    pub outer_leaf_hash: String,
    #[serde(rename = "leafIndex")]
    pub leaf_index: u64,
    #[serde(rename = "leafCount")]
    pub leaf_count: u64,
    #[serde(rename = "mmrItems")]
    pub mmr_items: Vec<String>,
    #[serde(rename = "simplifiedItems")]
    pub simplified_items: Vec<String>,
    #[serde(rename = "proofOrder")]
    pub proof_order: String,
    #[serde(rename = "mmrRoot")]
    pub mmr_root: String,
    #[serde(rename = "validatorCount")]
    pub validator_count: u16,
    #[serde(rename = "authorityKeys")]
    pub authority_keys: Vec<String>,
    #[serde(rename = "authorityAddresses")]
    pub authority_addresses: Vec<String>,
    #[serde(rename = "authorityRoot")]
    pub authority_root: String,
    #[serde(rename = "authorityProofs")]
    pub authority_proofs: Vec<Vec<String>>,
    #[serde(rename = "payloadItems")]
    pub payload_items: Vec<PayloadItem>,
    #[serde(rename = "signedCommitment")]
    pub signed_commitment: String,
    #[serde(rename = "commitmentBytes")]
    pub commitment_bytes: String,
    #[serde(rename = "commitmentHash")]
    pub commitment_hash: String,
    #[serde(rename = "queueProof")]
    pub queue_proof: String,
    #[serde(rename = "publicInputs")]
    pub public_inputs: String,
    #[serde(rename = "freshnessProof")]
    pub freshness_proof: FreshnessProof,
}

fn hex0x(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn hash_pair(left: Hash32, right: Hash32) -> Hash32 {
    let mut bytes = [0; 64];
    bytes[..32].copy_from_slice(&left);
    bytes[32..].copy_from_slice(&right);
    keccak256(&bytes)
}

fn build_mmr(leaves: &[Hash32]) -> (Hash32, BTreeMap<u64, Hash32>) {
    let mut nodes = BTreeMap::new();
    let mut size = 0u64;
    for leaf in leaves {
        let leaf_position = size;
        let mut position = size;
        let mut value = *leaf;
        let peak_map = mmr_lib::helper::get_peak_map(size);
        let mut peak = 1u64;
        while peak_map & peak != 0 {
            peak <<= 1;
            position += 1;
            let left_position = position - peak;
            value = hash_pair(*nodes.get(&left_position).expect("MMR left node"), value);
            nodes.insert(position, value);
        }
        nodes.insert(leaf_position, *leaf);
        size = position + 1;
    }
    let peaks = mmr_lib::helper::get_peaks(size);
    let mut root = *nodes
        .get(peaks.last().expect("nonempty MMR"))
        .expect("right peak");
    for position in peaks[..peaks.len() - 1].iter().rev() {
        root = hash_pair(root, *nodes.get(position).expect("left peak"));
    }
    (root, nodes)
}

fn native_items(leaf_index: u64, leaf_count: u64, nodes: &BTreeMap<u64, Hash32>) -> Vec<Hash32> {
    let size = 2 * leaf_count - leaf_count.count_ones() as u64;
    let peaks = mmr_lib::helper::get_peaks(size);
    let leaf_position = mmr_lib::leaf_index_to_pos(leaf_index);
    let target = peaks
        .iter()
        .position(|peak| leaf_position <= *peak)
        .expect("target peak");
    let target_peak = peaks[target];
    let mut items = peaks[..target]
        .iter()
        .map(|position| *nodes.get(position).expect("left peak"))
        .collect::<Vec<_>>();
    let mut position = leaf_position;
    let mut height = 0u8;
    while position != target_peak {
        let next_height = mmr_lib::helper::pos_height_in_tree(position + 1);
        let sibling_offset = (2u64 << height) - 1;
        let (sibling, parent) = if next_height > height {
            (position - sibling_offset, position + 1)
        } else {
            (position + sibling_offset, position + (2u64 << height))
        };
        items.push(*nodes.get(&sibling).expect("MMR sibling"));
        position = parent;
        height += 1;
    }
    if target + 1 < peaks.len() {
        let mut bagged = *nodes
            .get(peaks.last().expect("right peak"))
            .expect("right peak");
        for peak in peaks[target + 1..peaks.len() - 1].iter().rev() {
            bagged = hash_pair(bagged, *nodes.get(peak).expect("right peak"));
        }
        items.push(bagged);
    }
    items
}

fn deterministic_root(label: &str, force_gear_root: bool) -> Hash32 {
    if force_gear_root {
        return [0x11; 32];
    }
    let mut bytes = Vec::with_capacity(label.len() + 8);
    bytes.extend_from_slice(label.as_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    keccak256(&bytes)
}

fn signing_key(value: u16) -> SigningKey {
    let mut scalar = [0; 32];
    scalar[30..].copy_from_slice(&value.to_be_bytes());
    SigningKey::from_bytes((&scalar).into()).expect("development scalar is valid")
}

fn authority_keys(validator_count: u16) -> Result<Vec<[u8; 33]>> {
    ensure!(
        (1..=MAX_VALIDATORS as u16).contains(&validator_count),
        "authority count must be between 1 and {MAX_VALIDATORS}"
    );
    (1..=validator_count)
        .map(|value| {
            signing_key(value)
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .try_into()
                .map_err(|_| anyhow::anyhow!("compressed key has unexpected length"))
        })
        .collect()
}

fn timestamp_for(start: u64, source: u32) -> u64 {
    if source == 0 {
        0
    } else {
        BASE_SOURCE_TIMESTAMP_MS.saturating_add(
            u64::from(source)
                .saturating_sub(start)
                .saturating_mul(3_000),
        )
    }
}

fn payload_for(root: Hash32, label: &str) -> (Payload, Vec<PayloadItem>) {
    let extra = keccak256(format!("{label}:extra-payload").as_bytes());
    let entries = vec![(*b"aa", extra[..7].to_vec()), (*b"mh", root.to_vec())];
    let mut encoded = &entries.encode()[..];
    let payload = Payload::decode(&mut encoded).expect("canonical payload");
    let payload_items = entries
        .iter()
        .map(|(payload_id, data)| PayloadItem {
            payload_id: hex0x(payload_id),
            data: hex0x(data),
        })
        .collect();
    (payload, payload_items)
}

fn build_signed_commitment(
    payload: Payload,
    block: u32,
    validator_count: u16,
) -> (Vec<u8>, Vec<u8>, Hash32) {
    let commitment = Commitment {
        payload,
        block_number: block,
        validator_set_id: 0,
    };
    let digest = keccak256(&commitment.encode());
    let signatures: Vec<Option<sp_consensus_beefy::ecdsa_crypto::Signature>> = (1
        ..=validator_count)
        .map(|value| {
            let (signature, recovery_id) = signing_key(value)
                .sign_prehash_recoverable(&digest)
                .expect("sign development commitment");
            let mut raw = [0; 65];
            raw[..64].copy_from_slice(&signature.to_bytes());
            raw[64] = recovery_id.to_byte();
            Some(sp_consensus_beefy::ecdsa_crypto::Signature::unchecked_from(
                raw,
            ))
        })
        .collect();
    let signed = SignedCommitment {
        commitment,
        signatures,
    };
    let commitment_bytes = signed.commitment.encode();
    let versioned = VersionedFinalityProof::V1(signed).encode();
    (
        versioned,
        commitment_bytes.clone(),
        keccak256(&commitment_bytes),
    )
}

fn make_case(
    name: &str,
    (leaf_count, historical_index, start): (u64, u64, u64),
    initialized: bool,
    initialized_empty: bool,
    native_only: bool,
    validator_count: u16,
    golden_snapshot: bool,
) -> Result<InteropCase> {
    ensure!(
        leaf_count > historical_index,
        "fixture leaf index outside count"
    );
    ensure!(
        validator_count as usize <= MAX_VALIDATORS,
        "authority count too large"
    );
    let newest_index = leaf_count - 1;
    let insertion = start + historical_index;
    let source = u32::try_from(insertion - 1).expect("source block fits fixture");
    let anchor = start + leaf_count - 1;
    let source_genesis = if golden_snapshot {
        [0x99; 32]
    } else {
        keccak256(format!("{name}:genesis").as_bytes())
    };
    let source_domain = if golden_snapshot {
        [0x22; 32]
    } else {
        keccak256(format!("{name}:source-domain").as_bytes())
    };
    let destination_queue = [0x33; 20];
    let bridge_domain = crate::bridge_domain(source_domain, 31_337, destination_queue)?;
    let queue_id = if !initialized || initialized_empty {
        0
    } else if golden_snapshot {
        0x0102_0304_0506_0708
    } else {
        let bytes = keccak256(format!("{name}:queue-id").as_bytes());
        u64::from_le_bytes(bytes[..8].try_into().expect("queue id bytes")) | 1
    };
    let queue_root = if !initialized || initialized_empty {
        [0; 32]
    } else if golden_snapshot {
        [0x11; 32]
    } else {
        deterministic_root(name, false)
    };
    let historical_timestamp = if golden_snapshot {
        BASE_SOURCE_TIMESTAMP_MS
    } else {
        timestamp_for(start, source)
    };
    let historical_snapshot = QueueSnapshot::new(
        bridge_domain,
        historical_timestamp,
        initialized,
        queue_id,
        queue_root,
    )?;
    let freshness_source = u32::try_from(start + newest_index - 1).expect("freshness source fits");
    let freshness_timestamp = if native_only {
        historical_timestamp
    } else if golden_snapshot {
        BASE_SOURCE_TIMESTAMP_MS + 3_000
    } else {
        timestamp_for(start, freshness_source)
    };
    let freshness_queue_id = if native_only {
        queue_id
    } else if initialized && !initialized_empty {
        queue_id.saturating_add(u64::from(!golden_snapshot))
    } else {
        0
    };
    let freshness_root = if native_only {
        queue_root
    } else if initialized && !initialized_empty {
        keccak256(format!("{name}:fresh-root").as_bytes())
    } else {
        [0; 32]
    };
    let freshness_snapshot = QueueSnapshot::new(
        bridge_domain,
        freshness_timestamp,
        initialized,
        freshness_queue_id,
        freshness_root,
    )?;

    let keys = authority_keys(validator_count)?;
    let addresses = authority_addresses(&keys)?;
    let authority_set_root = authority_root(&addresses)?;
    let historical_parent_hash = keccak256(format!("{name}:parent:{source}").as_bytes());
    let freshness_parent_hash = keccak256(format!("{name}:parent:{freshness_source}").as_bytes());
    let historical_leaf = crate::make_leaf(
        source,
        historical_parent_hash,
        1,
        validator_count as u32,
        authority_set_root,
        historical_snapshot.hash(),
    );
    let freshness_leaf = crate::make_leaf(
        freshness_source,
        freshness_parent_hash,
        1,
        validator_count as u32,
        authority_set_root,
        freshness_snapshot.hash(),
    );
    let historical_leaf_bytes = encode_outer_leaf(&historical_leaf)?;
    let freshness_leaf_bytes = encode_outer_leaf(&freshness_leaf)?;
    let historical_leaf_hash = keccak256(&historical_leaf_bytes);
    let freshness_leaf_hash = keccak256(&freshness_leaf_bytes);
    let mut leaves = (0..leaf_count)
        .map(|index| keccak256(format!("{name}:mmr-leaf:{index}").as_bytes()))
        .collect::<Vec<_>>();
    leaves[historical_index as usize] = historical_leaf_hash;
    leaves[newest_index as usize] = freshness_leaf_hash;
    let (mmr_root, nodes) = build_mmr(&leaves);
    let historical_proof = RuntimeLeafProof {
        leaf_indices: vec![historical_index],
        leaf_count,
        items: native_items(historical_index, leaf_count, &nodes),
    };
    let freshness_proof = RuntimeLeafProof {
        leaf_indices: vec![newest_index],
        leaf_count,
        items: native_items(newest_index, leaf_count, &nodes),
    };
    ensure!(
        verify_native_mmr_proof(mmr_root, &historical_proof, historical_leaf_hash)?,
        "native historical fixture MMR proof failed for {name} (index {historical_index}, count {leaf_count}, items {})",
        historical_proof.items.len()
    );
    ensure!(
        verify_native_mmr_proof(mmr_root, &freshness_proof, freshness_leaf_hash)?,
        "native freshness fixture MMR proof failed"
    );
    let historical_simplified = convert_mmr_proof(&historical_proof)?;
    let freshness_simplified = convert_mmr_proof(&freshness_proof)?;
    ensure!(
        verify_simplified_mmr(mmr_root, historical_leaf_hash, &historical_simplified)?,
        "simplified historical fixture MMR proof failed for {name}"
    );
    ensure!(
        verify_simplified_mmr(mmr_root, freshness_leaf_hash, &freshness_simplified)?,
        "simplified freshness fixture MMR proof failed for {name}"
    );
    ensure!(
        historical_simplified.items.len() <= MAX_MMR_PROOF_ITEMS
            && freshness_simplified.items.len() <= MAX_MMR_PROOF_ITEMS,
        "fixture proof exceeds Solidity limit"
    );
    let (payload, payload_items) = payload_for(mmr_root, name);
    let (signed_bytes, commitment_bytes, commitment_hash) =
        build_signed_commitment(payload, anchor as u32, validator_count);
    let validated = validate_signed_commitment(&signed_bytes, &keys, Some(0), Some(anchor as u32))?;
    ensure!(
        validated.commitment_hash == commitment_hash && validated.mmr_root == mmr_root,
        "synthetic commitment failed cross-pin validation"
    );
    let queue_proof = encode_queue_proof(
        &historical_snapshot,
        anchor,
        mmr_root,
        &historical_leaf,
        &historical_simplified.items,
        historical_simplified.proof_order,
    )?;
    let public_inputs = encode_public_inputs(queue_root, source)?;
    let authority_proofs = addresses
        .iter()
        .enumerate()
        .map(|(index, _)| {
            binary_merkle_tree::merkle_proof::<KeccakHasher, _, _>(addresses.iter(), index)
                .proof
                .iter()
                .map(|item| hex0x(item))
                .collect()
        })
        .collect();
    let freshness = FreshnessProof {
        source_genesis: hex0x(&source_genesis),
        bridge_domain: hex0x(&bridge_domain),
        source_timestamp_ms: freshness_timestamp,
        initialized,
        queue_id: freshness_queue_id,
        queue_root: hex0x(&freshness_root),
        snapshot_preimage: hex0x(&freshness_snapshot.encode()),
        bridge_commitment: hex0x(&freshness_snapshot.hash()),
        outer_leaf: hex0x(&freshness_leaf_bytes),
        outer_leaf_hash: hex0x(&freshness_leaf_hash),
        leaf_index: newest_index,
        leaf_count,
        mmr_items: freshness_proof
            .items
            .iter()
            .map(|item| hex0x(item))
            .collect(),
        simplified_items: freshness_simplified
            .items
            .iter()
            .map(|item| hex0x(item))
            .collect(),
        proof_order: hex0x(&freshness_simplified.proof_order),
    };
    Ok(InteropCase {
        name: name.into(),
        native_only,
        mmr_start_block: start,
        source_block: source,
        insertion_block: insertion,
        anchor_block: anchor,
        queue_id,
        bridge_version: 2,
        source_genesis: hex0x(&source_genesis),
        source_domain: hex0x(&source_domain),
        destination_chain_id: 31_337,
        destination_queue: hex0x(&destination_queue),
        bridge_domain: hex0x(&bridge_domain),
        source_timestamp_ms: historical_timestamp,
        initialized,
        queue_root: hex0x(&queue_root),
        snapshot_preimage: hex0x(&historical_snapshot.encode()),
        bridge_commitment: hex0x(&historical_snapshot.hash()),
        outer_leaf: hex0x(&historical_leaf_bytes),
        outer_leaf_hash: hex0x(&historical_leaf_hash),
        leaf_index: historical_index,
        leaf_count,
        mmr_items: historical_proof
            .items
            .iter()
            .map(|item| hex0x(item))
            .collect(),
        simplified_items: historical_simplified
            .items
            .iter()
            .map(|item| hex0x(item))
            .collect(),
        proof_order: hex0x(&historical_simplified.proof_order),
        mmr_root: hex0x(&mmr_root),
        validator_count,
        authority_keys: keys.iter().map(|key| hex0x(key)).collect(),
        authority_addresses: addresses.iter().map(|address| hex0x(address)).collect(),
        authority_root: hex0x(&authority_set_root),
        authority_proofs,
        payload_items,
        signed_commitment: hex0x(&signed_bytes),
        commitment_bytes: hex0x(&commitment_bytes),
        commitment_hash: hex0x(&commitment_hash),
        queue_proof: hex0x(&queue_proof),
        public_inputs: hex0x(&public_inputs_bytes(&public_inputs)),
        freshness_proof: freshness,
    })
}

pub fn generate_fixtures() -> Result<InteropFixture> {
    Ok(InteropFixture {
        schema_version: 3,
        gear_baseline_commit: GEAR_BASELINE_COMMIT.into(),
        snowbridge_baseline_commit: SNOWBRIDGE_BASELINE_COMMIT.into(),
        cases: vec![
            make_case(
                "synthetic-mmr-3-primary",
                (3, 1, 100),
                true,
                false,
                false,
                3,
                true,
            )?,
            make_case(
                "synthetic-mmr-1-native-only",
                (1, 0, 1),
                true,
                false,
                true,
                3,
                false,
            )?,
            make_case(
                "synthetic-uninitialized",
                (3, 1, 1),
                false,
                false,
                false,
                3,
                false,
            )?,
            make_case(
                "synthetic-initialized-zero",
                (7, 5, 1),
                true,
                true,
                false,
                3,
                false,
            )?,
            make_case("synthetic-mmr-7", (7, 5, 1), true, false, false, 3, false)?,
            make_case(
                "synthetic-mmr-8-interior",
                (8, 6, 1),
                true,
                false,
                false,
                3,
                false,
            )?,
            make_case(
                "synthetic-mmr-15-rightmost",
                (15, 13, 1),
                true,
                false,
                false,
                3,
                false,
            )?,
            make_case(
                "synthetic-mmr-non-genesis",
                (61, 51, 1000),
                true,
                false,
                false,
                3,
                false,
            )?,
            make_case(
                "synthetic-authorities-1",
                (3, 1, 100),
                true,
                false,
                false,
                1,
                false,
            )?,
            make_case(
                "synthetic-authorities-2",
                (3, 1, 100),
                true,
                false,
                false,
                2,
                false,
            )?,
            make_case(
                "synthetic-authorities-59",
                (3, 1, 100),
                true,
                false,
                false,
                59,
                false,
            )?,
            make_case(
                "synthetic-authorities-150",
                (3, 1, 100),
                true,
                false,
                false,
                150,
                false,
            )?,
            make_case(
                "synthetic-authorities-256",
                (3, 1, 100),
                true,
                false,
                false,
                256,
                false,
            )?,
            make_case(
                "synthetic-authorities-4",
                (3, 1, 100),
                true,
                false,
                false,
                4,
                false,
            )?,
        ],
    })
}

pub fn fixture_json() -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&generate_fixtures()?)?
    ))
}

pub fn write_fixture(path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, fixture_json()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_fixtures_cover_required_shapes() {
        let fixture = generate_fixtures().expect("fixtures");
        assert_eq!(fixture.schema_version, 3);
        assert_eq!(fixture.cases[0].name, "synthetic-mmr-3-primary");
        assert!(
            fixture.cases[0].freshness_proof.source_timestamp_ms
                > fixture.cases[0].source_timestamp_ms
        );
        assert_eq!(fixture.cases[0].payload_items.len(), 2);
        assert_eq!(fixture.cases[0].authority_proofs.len(), 3);
        for count in [1, 3, 7, 8, 15] {
            assert!(fixture.cases.iter().any(|case| case.leaf_count == count));
        }
        for count in [1, 2, 3, 4, 59, 150, 256] {
            let case = fixture
                .cases
                .iter()
                .find(|case| case.validator_count == count)
                .expect("authority-size case");
            assert_eq!(case.authority_keys.len(), count as usize);
            assert_eq!(case.authority_proofs.len(), count as usize);
        }
        let non_genesis = fixture
            .cases
            .iter()
            .find(|case| case.name == "synthetic-mmr-non-genesis")
            .expect("non-genesis case");
        assert_eq!(
            (
                non_genesis.mmr_start_block,
                non_genesis.source_block,
                non_genesis.insertion_block,
                non_genesis.anchor_block,
                non_genesis.leaf_index,
                non_genesis.leaf_count
            ),
            (1000, 1050, 1051, 1060, 51, 61)
        );
    }

    #[test]
    fn checked_in_fixture_is_generated_by_this_code() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ethereum/test/fixtures/beefy-interop.json");
        if std::env::var_os("UPDATE_BEEFY_FIXTURE").is_some() {
            write_fixture(&path).expect("write fixture");
            return;
        }
        let checked_in = std::fs::read_to_string(path).expect("checked-in fixture");
        assert_eq!(checked_in, fixture_json().expect("fixture JSON"));
    }
}
