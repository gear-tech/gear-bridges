use anyhow::{ensure, Result};
use k256::ecdsa::SigningKey;
use parity_scale_codec::Encode;
use serde::{Deserialize, Serialize};
use sp_consensus_beefy::{Commitment, Payload, SignedCommitment, VersionedFinalityProof};
use sp_core::crypto::UncheckedFrom;
use sp_mmr_primitives::mmr_lib;
use std::{collections::BTreeMap, path::Path};

use crate::{
    protocol::{
        authority_addresses, authority_root, convert_mmr_proof, encode_outer_leaf,
        encode_public_inputs, encode_queue_proof, keccak256, public_inputs_bytes,
        verify_native_mmr_proof, verify_simplified_mmr, Hash32, QueueSnapshot, RuntimeLeafProof,
        MAX_MMR_PROOF_ITEMS,
    },
    validate_signed_commitment,
};

const GEAR_COMMIT: &str = "0b13f2c61b0e5d9844c7efd12727487a2fdb8c63";
const SNOWBRIDGE_COMMIT: &str = "1201293e482ef052b9c3989dcf680046704fef3d";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteropFixture {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "gearCommit")]
    pub gear_commit: String,
    #[serde(rename = "snowbridgeCommit")]
    pub snowbridge_commit: String,
    pub cases: Vec<InteropCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteropCase {
    pub name: String,
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
    #[serde(rename = "authorityKeys")]
    pub authority_keys: Vec<String>,
    #[serde(rename = "authorityAddresses")]
    pub authority_addresses: Vec<String>,
    #[serde(rename = "authorityRoot")]
    pub authority_root: String,
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
}

fn hex0x(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn signing_key(value: u8) -> SigningKey {
    SigningKey::from_bytes((&[value; 32]).into()).expect("development scalar is valid")
}

fn authority_keys() -> Vec<[u8; 33]> {
    (1u8..=3)
        .map(|value| {
            signing_key(value)
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .try_into()
                .expect("compressed key")
        })
        .collect()
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

fn build_signed_commitment(root: Hash32, block: u32) -> (Vec<u8>, Vec<u8>, Hash32) {
    let commitment = Commitment {
        payload: Payload::from_single_entry(*b"mh", root.to_vec()),
        block_number: block,
        validator_set_id: 0,
    };
    let digest = keccak256(&commitment.encode());
    let signatures = (1u8..=3)
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
    leaf_count: u64,
    leaf_index: u64,
    start: u64,
    snapshot_version: u8,
    force_gear_root: bool,
) -> Result<InteropCase> {
    ensure!(leaf_count > leaf_index, "fixture leaf index outside count");
    let insertion = start + leaf_index;
    let source = u32::try_from(insertion - 1).expect("source block fits fixture");
    let anchor = start + leaf_count - 1;
    let queue_id = if snapshot_version == 255 {
        0
    } else if force_gear_root {
        0x0102_0304_0506_0708
    } else {
        0x8877_6655_4433_2211 ^ leaf_count
    };
    let queue_root = if snapshot_version == 255 || name == "synthetic-initialized-zero" {
        [0; 32]
    } else {
        deterministic_root(name, force_gear_root)
    };
    let snapshot = QueueSnapshot::new(snapshot_version, queue_id, queue_root);
    let bridge_commitment = snapshot.hash();
    let keys = authority_keys();
    let addresses = authority_addresses(&keys)?;
    let authority_set_root = authority_root(&addresses)?;
    let parent_hash = keccak256(format!("{name}:parent:{source}").as_bytes());
    let leaf = crate::make_leaf(
        source,
        parent_hash,
        1,
        keys.len() as u32,
        authority_set_root,
        bridge_commitment,
    );
    let leaf_bytes = encode_outer_leaf(&leaf)?;
    let leaf_hash = keccak256(&leaf_bytes);
    let mut leaves = (0..leaf_count)
        .map(|index| keccak256(format!("{name}:mmr-leaf:{index}").as_bytes()))
        .collect::<Vec<_>>();
    leaves[leaf_index as usize] = leaf_hash;
    let (mmr_root, nodes) = build_mmr(&leaves);
    let proof = RuntimeLeafProof {
        leaf_indices: vec![leaf_index],
        leaf_count,
        items: native_items(leaf_index, leaf_count, &nodes),
    };
    ensure!(
        verify_native_mmr_proof(mmr_root, &proof, leaf_hash)?,
        "native fixture MMR proof failed"
    );
    let simplified = convert_mmr_proof(&proof)?;
    ensure!(
        verify_simplified_mmr(mmr_root, leaf_hash, &simplified)?,
        "simplified fixture MMR proof failed for {name}"
    );
    ensure!(
        simplified.items.len() <= MAX_MMR_PROOF_ITEMS,
        "fixture proof exceeds Solidity limit"
    );
    let (signed_bytes, commitment_bytes, commitment_hash) =
        build_signed_commitment(mmr_root, anchor as u32);
    let validated = validate_signed_commitment(&signed_bytes, &keys, Some(0), Some(anchor as u32))?;
    ensure!(
        validated.commitment_hash == commitment_hash && validated.mmr_root == mmr_root,
        "synthetic commitment failed cross-pin validation"
    );
    let queue_proof = encode_queue_proof(
        0,
        0,
        queue_id,
        anchor,
        mmr_root,
        &leaf,
        &simplified.items,
        simplified.proof_order,
    )?;
    let public_inputs = encode_public_inputs(queue_root, source)?;
    Ok(InteropCase {
        name: name.into(),
        mmr_start_block: start,
        source_block: source,
        insertion_block: insertion,
        anchor_block: anchor,
        queue_id,
        bridge_version: snapshot_version,
        queue_root: hex0x(&queue_root),
        snapshot_preimage: hex0x(&snapshot.encode()),
        bridge_commitment: hex0x(&bridge_commitment),
        outer_leaf: hex0x(&leaf_bytes),
        outer_leaf_hash: hex0x(&leaf_hash),
        leaf_index,
        leaf_count,
        mmr_items: proof.items.iter().map(|item| hex0x(item)).collect(),
        simplified_items: simplified.items.iter().map(|item| hex0x(item)).collect(),
        proof_order: hex0x(&simplified.proof_order),
        mmr_root: hex0x(&mmr_root),
        authority_keys: keys.iter().map(|key| hex0x(key)).collect(),
        authority_addresses: addresses.iter().map(|address| hex0x(address)).collect(),
        authority_root: hex0x(&authority_set_root),
        signed_commitment: hex0x(&signed_bytes),
        commitment_bytes: hex0x(&commitment_bytes),
        commitment_hash: hex0x(&commitment_hash),
        queue_proof: hex0x(&queue_proof),
        public_inputs: hex0x(&public_inputs_bytes(&public_inputs)),
    })
}

pub fn generate_fixtures() -> Result<InteropFixture> {
    Ok(InteropFixture {
        schema_version: 1,
        gear_commit: GEAR_COMMIT.into(),
        snowbridge_commit: SNOWBRIDGE_COMMIT.into(),
        cases: vec![
            make_case("synthetic-initialized-gear-pin", 1, 0, 1, 0, true)?,
            make_case("synthetic-uninitialized", 3, 1, 1, 255, false)?,
            make_case("synthetic-initialized-zero", 7, 3, 1, 0, false)?,
            make_case("synthetic-mmr-3", 3, 1, 1, 0, false)?,
            make_case("synthetic-mmr-7", 7, 3, 1, 0, false)?,
            make_case("synthetic-mmr-8-interior", 8, 4, 1, 0, false)?,
            make_case("synthetic-mmr-15-rightmost", 15, 14, 1, 0, false)?,
            make_case("synthetic-mmr-non-genesis", 61, 51, 1000, 0, false)?,
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
        let counts: Vec<_> = fixture.cases.iter().map(|case| case.leaf_count).collect();
        assert!(counts.contains(&1));
        assert!(counts.contains(&3));
        assert!(counts.contains(&7));
        assert!(counts.contains(&8));
        assert!(counts.contains(&15));
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
