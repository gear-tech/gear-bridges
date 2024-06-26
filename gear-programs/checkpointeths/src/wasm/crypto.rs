use super::*;
use ark_bls12_381::{Bls12_381, G1Affine, G2Affine};
use ark_ec::{bls12::Bls12Config, pairing::Pairing, AffineRepr, CurveGroup};
use ark_ff::{fields::field_hashers::DefaultFieldHasher, Field, Zero};
use ark_serialize::CanonicalSerialize;
use core::ops::Neg;
use gbuiltin_bls381::*;
use gstd::ActorId;
use io::{tree_hash, ArkScale};
use parity_scale_codec::{Decode, Encode};
use tree_hash::TreeHash;

const BUILTIN_BLS381: ActorId = ActorId::new(hex_literal::hex!(
    "6b6e292c382945e80bf51af2ba7fe9f458dcff81ae6075c46f9095e1bbecdc37"
));

pub async fn verify_sync_committee_signature(
    genesis: &Genesis,
    pub_keys: Vec<G1>,
    attested_header: &BeaconBlockHeader,
    signature: &G2,
    signature_slot: u64,
) -> bool {
    let H256(header_root) = attested_header.tree_hash_root();
    let signing_root = compute_committee_sign_root(genesis, header_root, signature_slot);

    let points: ArkScale<Vec<G1>> = pub_keys.into();
    let request = Request::AggregateG1 {
        points: points.encode(),
    }
    .encode();
    let reply = msg::send_bytes_for_reply(BUILTIN_BLS381, &request, 0, 0)
        .expect("Failed to send message")
        .await
        .expect("Received error reply");
    let response = Response::decode(&mut reply.as_slice())
        .expect("Aggregate G1 reply should be properly encoded");
    let pub_key_aggregated = match response {
        Response::AggregateG1(v) => ArkScale::<G1>::decode(&mut v.as_slice())
            .expect("Aggregate G1 result should properly encoded"),
        _ => unreachable!(),
    };

    // Ensure AggregatePublicKey is not at infinity
    if pub_key_aggregated.0.is_zero() {
        return false;
    }

    let request = Request::MapToG2Affine {
        message: signing_root.as_ref().to_vec(),
    }
    .encode();
    let reply = msg::send_bytes_for_reply(BUILTIN_BLS381, &request, 0, 0)
        .expect("Failed to send message")
        .await
        .expect("Received error reply");
    let response = Response::decode(&mut reply.as_slice())
        .expect("MapToG2Affine reply should be properly encoded");
    let message = match response {
        Response::MapToG2Affine(v) => ArkScale::<G2Affine>::decode(&mut v.as_slice())
            .expect("MapToG2Affine result should properly encoded"),
        _ => unreachable!(),
    };

    let pub_key: G1Affine = From::from(pub_key_aggregated.0);
    let signature: G2Affine = From::from(*signature);
    let generator_g1_negative = G1Affine::generator().neg();

    // pairing
    let a: ArkScale<Vec<G1Affine>> = vec![generator_g1_negative, pub_key].into();
    let b: ArkScale<Vec<G2Affine>> = vec![signature, message.0].into();
    let request = Request::MultiMillerLoop {
        a: a.encode(),
        b: b.encode(),
    }
    .encode();
    let reply = msg::send_bytes_for_reply(BUILTIN_BLS381, &request, 0, 0)
        .expect("Failed to send message")
        .await
        .expect("Received error reply");
    let response = Response::decode(&mut reply.as_slice()).unwrap();
    let miller_loop = match response {
        Response::MultiMillerLoop(v) => v,
        _ => unreachable!(),
    };

    let request = Request::FinalExponentiation { f: miller_loop }.encode();
    let reply = msg::send_bytes_for_reply(BUILTIN_BLS381, &request, 0, 0)
        .expect("Failed to send message")
        .await
        .expect("Received error reply");
    let response = Response::decode(&mut reply.as_slice()).unwrap();
    let exp = match response {
        Response::FinalExponentiation(v) => {
            ArkScale::<<Bls12_381 as Pairing>::TargetField>::decode(&mut v.as_slice()).unwrap()
        }
        _ => unreachable!(),
    };

    <Bls12_381 as Pairing>::TargetField::ONE == exp.0
}

#[derive(Default, Debug, tree_hash_derive::TreeHash)]
struct SigningData {
    object_root: [u8; 32],
    domain: [u8; 32],
}

#[derive(Default, Debug, tree_hash_derive::TreeHash)]
struct ForkData {
    current_version: [u8; 4],
    genesis_validator_root: [u8; 32],
}

pub fn compute_signing_root(object_root: [u8; 32], domain: [u8; 32]) -> Hash256 {
    let data = SigningData {
        object_root,
        domain,
    };

    data.tree_hash_root()
}

pub fn compute_domain(
    domain_type: &[u8],
    fork_version: [u8; 4],
    genesis_root: [u8; 32],
) -> [u8; 32] {
    let H256(fork_data_root) = compute_fork_data_root(fork_version, genesis_root);
    let start = domain_type;
    let end = &fork_data_root.as_ref()[..28];
    let d = [start, end].concat();

    d.to_vec().try_into().unwrap()
}

fn compute_fork_data_root(current_version: [u8; 4], genesis_validator_root: [u8; 32]) -> Hash256 {
    let fork_data = ForkData {
        current_version,
        genesis_validator_root,
    };

    fork_data.tree_hash_root()
}

fn compute_committee_sign_root(genesis: &Genesis, header: [u8; 32], _slot: u64) -> Hash256 {
    let H256(genesis_root) = genesis.validators_root();

    let domain_type = [0x07, 0x00, 0x00, 0x00];

    // Deneb
    let fork_version = hex_literal::hex!("90000073");
    let domain = compute_domain(&domain_type, fork_version, genesis_root);

    compute_signing_root(header, domain)
}
