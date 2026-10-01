use crate::{fixtures::generate_fixtures, protocol::*};
use parity_scale_codec::{Decode, Encode};
use sp_consensus_beefy::VersionedFinalityProof;
use sp_core::crypto::UncheckedFrom;

fn bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex.strip_prefix("0x").unwrap()).unwrap()
}

fn hash(hex: &str) -> Hash32 {
    bytes(hex).try_into().unwrap()
}

#[test]
fn full_width_public_inputs_preserve_root_and_height() {
    let root = std::array::from_fn(|i| i as u8 + 1);
    let [p0, p1] = encode_public_inputs(root, 0xa1b2c3d4).unwrap();
    assert_eq!(&p0[..8], &[0; 8]);
    assert_eq!(&p1[..8], &[0; 8]);
    assert_eq!(&p1[20..], &[0; 12]);
    let mut restored = [0; 32];
    restored[..24].copy_from_slice(&p0[8..]);
    restored[24..].copy_from_slice(&p1[8..16]);
    assert_eq!(restored, root);
    assert_eq!(
        u32::from_be_bytes(p1[16..20].try_into().unwrap()),
        0xa1b2c3d4
    );
}

#[test]
fn native_and_simplified_proofs_reject_corruption() {
    for case in generate_fixtures().unwrap().cases {
        let root = hash(&case.mmr_root);
        let leaf = hash(&case.outer_leaf_hash);
        let proof = RuntimeLeafProof {
            leaf_indices: vec![case.leaf_index],
            leaf_count: case.leaf_count,
            items: case.mmr_items.iter().map(|h| hash(h)).collect(),
        };
        assert!(verify_native_mmr_proof(root, &proof, leaf).unwrap());
        let simplified = convert_mmr_proof(&proof).unwrap();
        assert!(verify_simplified_mmr(root, leaf, &simplified).unwrap());
        let mut wrong_leaf = leaf;
        wrong_leaf[0] ^= 1;
        assert!(!verify_native_mmr_proof(root, &proof, wrong_leaf).unwrap());
        assert!(!verify_simplified_mmr(root, wrong_leaf, &simplified).unwrap());
        let mut extra = proof.clone();
        extra.items.push([0; 32]);
        assert!(convert_mmr_proof(&extra).is_err());
        if !proof.items.is_empty() {
            let mut missing = proof.clone();
            missing.items.pop();
            assert!(convert_mmr_proof(&missing).is_err());
            let mut changed = simplified.clone();
            changed.items[0][0] ^= 1;
            assert!(!verify_simplified_mmr(root, leaf, &changed).unwrap());
            let mut reordered = simplified.clone();
            reordered.proof_order[31] ^= 1;
            assert!(!verify_simplified_mmr(root, leaf, &reordered).unwrap());
        }
        let mut padding = simplified;
        padding.proof_order[0] |= 128;
        assert!(verify_simplified_mmr(root, leaf, &padding).is_err());
        let mut invalid = proof.clone();
        invalid.leaf_count = 0;
        assert!(convert_mmr_proof(&invalid).is_err());
        invalid = proof.clone();
        invalid.leaf_indices = vec![proof.leaf_count];
        assert!(convert_mmr_proof(&invalid).is_err());
        invalid.leaf_indices = vec![0, 1];
        assert!(convert_mmr_proof(&invalid).is_err());
        if proof.leaf_count > 1 {
            invalid = proof.clone();
            invalid.leaf_indices[0] = (case.leaf_index + 1) % case.leaf_count;
            assert!(!verify_native_mmr_proof(root, &invalid, leaf).unwrap_or(false));
            invalid = proof.clone();
            invalid.leaf_count += 1;
            assert!(validate_mmr_coordinates(
                &invalid,
                case.mmr_start_block,
                case.source_block,
                case.anchor_block
            )
            .is_err());
        }
    }
    assert!(leaf_index_to_position(u64::MAX / 2 + 1).is_err());
    assert!(convert_mmr_proof(&RuntimeLeafProof {
        leaf_indices: vec![0],
        leaf_count: 1,
        items: vec![[0; 32]; 257]
    })
    .is_err());
}

#[test]
fn signatures_are_canonical_positional_and_quorate() {
    let fixture = generate_fixtures().unwrap().cases.remove(0);
    let raw = bytes(&fixture.signed_commitment);
    let keys: Vec<[u8; 33]> = fixture
        .authority_keys
        .iter()
        .map(|key| bytes(key).try_into().unwrap())
        .collect();
    let signed = validate_signed_commitment(&raw, &keys, Some(0), Some(1)).unwrap();
    assert_eq!(signed.signed_indices, vec![0, 1, 2]);
    assert_eq!(signed.commitment_hash, hash(&fixture.commitment_hash));
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(decode_versioned_finality_proof(&trailing).is_err());
    let mut version = raw.clone();
    version[0] = 255;
    assert!(decode_versioned_finality_proof(&version).is_err());
    assert!(decode_versioned_finality_proof(&raw[..raw.len() - 1]).is_err());
    let mut duplicate = keys.clone();
    duplicate[1] = duplicate[0];
    assert!(validate_signed_commitment(&raw, &duplicate, None, None).is_err());
    assert!(authority_address(&[0; 33]).is_err());
    assert!(validate_signed_commitment(&raw, &keys, Some(1), None).is_err());
    assert!(validate_signed_commitment(&raw, &keys, None, Some(2)).is_err());
    for mutation in 0..5 {
        let mut bad = signed.signed.clone();
        match mutation {
            0 => bad.signatures.swap(0, 1),
            1 => bad.signatures[0] = None,
            2 => bad.commitment.block_number += 1,
            3 | 4 => {
                let mut signature = [0; 65];
                signature.copy_from_slice(bad.signatures[0].as_ref().unwrap().as_ref());
                if mutation == 3 {
                    signature[64] = 2;
                } else {
                    let order = sp_core::U256::from_big_endian(
                        &hex::decode(
                            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
                        )
                        .unwrap(),
                    );
                    let s = sp_core::U256::from_big_endian(&signature[32..64]);
                    (order - s).to_big_endian(&mut signature[32..64]);
                    signature[64] ^= 1;
                }
                bad.signatures[0] = Some(
                    sp_consensus_beefy::ecdsa_crypto::Signature::unchecked_from(signature),
                );
            }
            _ => unreachable!(),
        }
        assert!(validate_signed_commitment(
            &VersionedFinalityProof::V1(bad).encode(),
            &keys,
            None,
            None
        )
        .is_err());
    }
}

#[test]
fn signed_payloads_require_one_canonical_mmr_root() {
    use sp_core::Pair;
    let fixture = generate_fixtures().unwrap().cases.remove(0);
    let keys: Vec<[u8; 33]> = fixture
        .authority_keys
        .iter()
        .map(|key| bytes(key).try_into().unwrap())
        .collect();
    let original = decode_versioned_finality_proof(&bytes(&fixture.signed_commitment)).unwrap();
    for entries in [
        vec![],
        vec![(*b"xx", vec![0; 32])],
        vec![(*b"mh", vec![0; 31])],
        vec![(*b"mh", vec![0; 33])],
        vec![(*b"mh", vec![0; 32]), (*b"mh", vec![0; 32])],
        vec![(*b"zz", vec![]), (*b"mh", vec![0; 32])],
    ] {
        let mut signed = original.clone();
        signed.commitment.payload =
            sp_consensus_beefy::Payload::decode(&mut &entries.encode()[..]).unwrap();
        let digest = keccak256(&signed.commitment.encode());
        signed.signatures = (1..=3)
            .map(|key| {
                let signature = sp_core::ecdsa::Pair::from_seed(&[key; 32]).sign_prehashed(&digest);
                Some(sp_consensus_beefy::ecdsa_crypto::Signature::unchecked_from(
                    signature.0,
                ))
            })
            .collect();
        assert!(validate_signed_commitment(
            &VersionedFinalityProof::V1(signed).encode(),
            &keys,
            None,
            None
        )
        .is_err());
    }
}
