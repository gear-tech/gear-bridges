use ark_bls12_381::{Bls12_381, G1Affine, G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::{Field, Zero};
use core::ops::Neg;
use gbuiltin_bls381::*;
use checkpoint_light_client_io::{ArkScale, G1, G2};
use ethereum_common::{network::Network, tree_hash::TreeHash, beacon::BlockHeader as BeaconBlockHeader, signing_root, DOMAIN_SYNC_COMMITTEE};
use sails_rs::{prelude::*, gstd::msg};

const BUILTIN_BLS381: ActorId = ActorId::new(hex_literal::hex!(
    "6b6e292c382945e80bf51af2ba7fe9f458dcff81ae6075c46f9095e1bbecdc37"
));

pub async fn verify_sync_committee_signature(
    network: &Network,
    pub_keys: Vec<G1>,
    attested_header: &BeaconBlockHeader,
    signature: &G2,
    _signature_slot: u64,
) -> bool {
    let domain = signing_root::compute_domain(
        DOMAIN_SYNC_COMMITTEE,
        network.fork_version(),
        network.genesis_validators_root(),
    );
    let signing_root = signing_root::compute(attested_header.tree_hash_root(), domain);

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
