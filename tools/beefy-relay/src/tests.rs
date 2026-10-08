use crate::{fixtures::generate_fixtures, protocol::*};
use k256::ecdsa::SigningKey;
use parity_scale_codec::{Decode, Encode};
use sp_consensus_beefy::{Payload, VersionedFinalityProof};
use sp_core::crypto::UncheckedFrom;

fn bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex.strip_prefix("0x").unwrap()).unwrap()
}

fn hash(hex: &str) -> Hash32 {
    bytes(hex).try_into().unwrap()
}

fn scalar_key(value: u16) -> SigningKey {
    let mut scalar = [0; 32];
    scalar[30..].copy_from_slice(&value.to_be_bytes());
    SigningKey::from_bytes((&scalar).into()).expect("valid scalar")
}

fn resign(
    mut signed: sp_consensus_beefy::SignedCommitment<
        u32,
        sp_consensus_beefy::ecdsa_crypto::Signature,
    >,
    entries: Vec<([u8; 2], Vec<u8>)>,
) -> Vec<u8> {
    signed.commitment.payload = Payload::decode(&mut &entries.encode()[..]).expect("payload");
    let digest = keccak256(&signed.commitment.encode());
    let validator_count = signed.signatures.len() as u16;
    signed.signatures = (1..=validator_count)
        .map(|value| {
            let (signature, recovery_id) = scalar_key(value)
                .sign_prehash_recoverable(&digest)
                .expect("signature");
            let mut raw = [0; 65];
            raw[..64].copy_from_slice(&signature.to_bytes());
            raw[64] = recovery_id.to_byte();
            Some(sp_consensus_beefy::ecdsa_crypto::Signature::unchecked_from(
                raw,
            ))
        })
        .collect();
    VersionedFinalityProof::V1(signed).encode()
}

#[test]
fn bounded_scale_parsers_reject_lengths_before_decoding() {
    use parity_scale_codec::Compact;
    use sp_consensus_beefy::{
        ecdsa_crypto::{AuthorityId, Signature},
        ConsensusLog,
    };
    use sp_runtime::generic::DigestItem;
    let huge = Compact(u32::MAX).encode();
    assert!(decode_authority_list(&huge).is_err());
    assert!(decode_validator_set(&[vec![1], huge.clone()].concat()).is_err());
    assert!(decode_mmr_leaves(&huge).is_err());
    assert!(decode_mmr_proof(&huge).is_err());
    let leaf = encode_outer_leaf(&make_leaf(100, [1; 32], 1, 256, [2; 32], [3; 32])).unwrap();
    assert_eq!(
        decode_mmr_leaves(&vec![leaf.clone()].encode()).unwrap(),
        leaf
    );
    let mut leaves = vec![leaf].encode();
    leaves.push(0);
    assert!(decode_mmr_leaves(&leaves).is_err());
    assert!(decode_mmr_leaves(&[Compact(1u32).encode(), huge.clone()].concat()).is_err());
    let proof = RuntimeLeafProof {
        leaf_indices: vec![1],
        leaf_count: 3,
        items: vec![[4; 32]; 256],
    };
    assert_eq!(decode_mmr_proof(&proof.encode()).unwrap(), proof);
    let mut invalid = proof.clone();
    invalid.items.push([4; 32]);
    assert!(decode_mmr_proof(&invalid.encode()).is_err());
    invalid = proof.clone();
    invalid.leaf_indices.push(2);
    assert!(decode_mmr_proof(&invalid.encode()).is_err());
    let mut trailing = proof.encode();
    trailing.push(0);
    assert!(decode_mmr_proof(&trailing).is_err());
    let root: ConsensusLog<AuthorityId> = ConsensusLog::MmrRoot([4; 32].into());
    let digest = DigestItem::Consensus(*b"BEEF", root.encode()).encode();
    assert_eq!(decode_beefy_digest(&digest).unwrap(), Some([4; 32]));
    let mut trailing = digest;
    trailing.push(0);
    assert!(decode_beefy_digest(&trailing).is_err());
    let oversized_authorities =
        DigestItem::Consensus(*b"BEEF", [vec![1], huge.clone()].concat()).encode();
    assert!(decode_beefy_digest(&oversized_authorities).is_err());
    assert!(decode_beefy_digest(&[vec![4], b"BEEF".to_vec(), huge.clone()].concat()).is_err());
    let fixture = generate_fixtures().unwrap();
    let signed =
        decode_versioned_finality_proof(&bytes(&fixture.cases[0].signed_commitment)).unwrap();
    let commitment = signed.commitment;
    let compact = |bits: Vec<u8>, n: u32, signatures: Vec<Signature>| {
        (1u8, commitment.clone(), bits, n, signatures).encode()
    };
    let signatures: Vec<Signature> = signed.signatures.into_iter().flatten().collect();
    assert!(decode_versioned_finality_proof(&compact(vec![0xe0], 3, signatures.clone())).is_ok());
    for raw in [
        compact(vec![0xe1], 3, signatures.clone()), // Nonzero low padding bit.
        compact(vec![0xe0, 0], 3, signatures.clone()), // Extra bitfield byte.
        compact(vec![0xe0], 257, signatures.clone()),
        compact(vec![0xe0], u32::MAX, signatures.clone()),
        compact(vec![0xe0], 3, signatures[..2].to_vec()), // Missing set bit's signature.
        compact(vec![0xe0], 3, vec![signatures[0].clone(); 4]), // Unused extra signature.
        vec![0; MAX_BEEFY_PROOF_BYTES + 1],
        [vec![1], huge.clone()].concat(), // Payload entry count.
        [vec![1], Compact(1u32).encode(), b"mh".to_vec(), huge].concat(), // Payload body length.
    ] {
        assert!(decode_versioned_finality_proof(&raw).is_err());
    }
    let key: [u8; 33] = scalar_key(1)
        .verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .try_into()
        .unwrap();
    let authorities = vec![sp_consensus_beefy::ecdsa_crypto::AuthorityId::unchecked_from(key); 257];
    assert!(decode_authority_list(&authorities.encode()).is_err());
    let set = sp_consensus_beefy::ValidatorSet::new(authorities, 0).unwrap();
    assert!(decode_validator_set(&Some(set).encode()).is_err());
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
        items: vec![[0; 32]; 257],
    })
    .is_err());
}

#[test]
fn signatures_are_canonical_positional_and_quorate() {
    let fixture = generate_fixtures().unwrap();
    let primary = &fixture.cases[0];
    let raw = bytes(&primary.signed_commitment);
    let keys: Vec<[u8; 33]> = primary
        .authority_keys
        .iter()
        .map(|key| bytes(key).try_into().unwrap())
        .collect();
    let signed =
        validate_signed_commitment(&raw, &keys, Some(0), Some(primary.anchor_block as u32))
            .unwrap();
    assert_eq!(signed.signed_indices, vec![0, 1, 2]);
    assert_eq!(signed.commitment_bytes, bytes(&primary.commitment_bytes));
    assert_eq!(signed.commitment_hash, hash(&primary.commitment_hash));
    let mut undersigned = decode_versioned_finality_proof(&raw).unwrap();
    undersigned.signatures[2] = None;
    let undersigned = VersionedFinalityProof::V1(undersigned).encode();
    assert!(validate_signed_commitment(
        &undersigned,
        &keys,
        Some(0),
        Some(primary.anchor_block as u32)
    )
    .is_err());
    for case in fixture
        .cases
        .iter()
        .filter(|case| [2, 3, 4, 59, 150, 256].contains(&case.validator_count))
    {
        let keys: Vec<[u8; 33]> = case
            .authority_keys
            .iter()
            .map(|key| bytes(key).try_into().unwrap())
            .collect();
        let signed = validate_signed_commitment(
            &bytes(&case.signed_commitment),
            &keys,
            Some(0),
            Some(case.anchor_block as u32),
        )
        .unwrap();
        assert_eq!(
            signed.signed_indices.len(),
            usize::from(case.validator_count)
        );
        assert_eq!(
            signed.signed_indices.last().copied(),
            Some((usize::from(case.validator_count) - 1) as u32)
        );
        let quorum = keys.len() - (keys.len() - 1) / 3;
        if keys.len() == 256 {
            assert_eq!(quorum, 171);
        }
        if keys.len() == 59 {
            assert_eq!(quorum, 40);
        }
        let mut minimal = signed.signed.clone();
        for signature in minimal.signatures.iter_mut().skip(quorum) {
            *signature = None;
        }
        assert!(validate_signed_commitment(
            &VersionedFinalityProof::V1(minimal.clone()).encode(),
            &keys,
            None,
            None
        )
        .is_ok());
        minimal.signatures[quorum - 1] = None;
        assert!(validate_signed_commitment(
            &VersionedFinalityProof::V1(minimal).encode(),
            &keys,
            None,
            None
        )
        .is_err());
        let mut duplicate = signed.signed.clone();
        duplicate.signatures[1] = duplicate.signatures[0].clone();
        assert!(validate_signed_commitment(
            &VersionedFinalityProof::V1(duplicate).encode(),
            &keys,
            None,
            None
        )
        .is_err());
    }
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
}

#[test]
fn signed_payloads_require_one_canonical_mmr_root_and_preserve_extra_items() {
    let fixture = &generate_fixtures().unwrap().cases[0];
    let keys: Vec<[u8; 33]> = fixture
        .authority_keys
        .iter()
        .map(|key| bytes(key).try_into().unwrap())
        .collect();
    let original = decode_versioned_finality_proof(&bytes(&fixture.signed_commitment)).unwrap();
    let valid_entries = vec![(*b"aa", vec![7, 8, 9]), (*b"mh", bytes(&fixture.mmr_root))];
    let valid = resign(original.clone(), valid_entries);
    let validated = validate_signed_commitment(&valid, &keys, None, None).unwrap();
    assert_eq!(validated.mmr_root, hash(&fixture.mmr_root));
    for entries in [
        vec![],
        vec![(*b"xx", vec![0; 32])],
        vec![(*b"mh", vec![0; 31])],
        vec![(*b"mh", vec![0; 33])],
        vec![(*b"mh", vec![0; 32]), (*b"mh", vec![0; 32])],
        vec![(*b"aa", vec![0; 1]), (*b"mh", vec![0; 32])],
    ] {
        let bad = resign(original.clone(), entries);
        assert!(validate_signed_commitment(&bad, &keys, None, None).is_err());
    }
}

#[test]
fn authority_proofs_cover_positions_and_size_bounds() {
    let fixture = generate_fixtures().unwrap();
    for case in fixture
        .cases
        .iter()
        .filter(|case| case.validator_count >= 59)
    {
        let keys: Vec<[u8; 33]> = case
            .authority_keys
            .iter()
            .map(|key| bytes(key).try_into().unwrap())
            .collect();
        let addresses = authority_addresses(&keys).unwrap();
        let root = hash(&case.authority_root);
        assert_eq!(authority_root(&addresses).unwrap(), root);
        for index in [0, keys.len() / 2, keys.len() - 1] {
            let proof = authority_proof(&keys, index).unwrap();
            assert!(verify_authority_proof(&root, &proof, keys.len()));
            assert_eq!(
                proof.siblings,
                case.authority_proofs[index]
                    .iter()
                    .map(|item| hash(item))
                    .collect::<Vec<_>>()
            );
        }
    }
    assert!(authority_addresses(&[]).is_err());
    assert!(authority_addresses(&vec![[2; 33]; MAX_VALIDATORS + 1]).is_err());
}

#[test]
fn queue_envelopes_match_v2_fixture_geometry() {
    let case = &generate_fixtures().unwrap().cases[0];
    let snapshot = QueueSnapshot::decode(&bytes(&case.snapshot_preimage)).unwrap();
    let leaf = decode_outer_leaf(&bytes(&case.outer_leaf)).unwrap();
    let order: [u8; 32] = bytes(&case.proof_order).try_into().unwrap();
    let items: Vec<Hash32> = case
        .simplified_items
        .iter()
        .map(|item| hash(item))
        .collect();
    let proof = encode_queue_proof(
        &snapshot,
        case.anchor_block,
        hash(&case.mmr_root),
        &leaf,
        &items,
        order,
    )
    .unwrap();
    assert_eq!(proof, bytes(&case.queue_proof));
    assert_eq!(proof.len(), 576 + 32 * items.len());
    assert!(proof[..31].iter().all(|byte| *byte == 0));
    assert_eq!(proof[31], 2);
}
