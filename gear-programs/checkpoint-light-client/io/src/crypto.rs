use crate::{
    verification::{BackendError, BlsBackend, Request, Response},
    ArkScale, G1, G2,
};
use alloc::{vec, vec::Vec};
use ark_bls12_381::{Bls12_381, G1Affine, G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::{Field, Zero};
use ark_serialize::Valid;
use core::ops::Neg;
use ethereum_common::{
    beacon::BlockHeader as BeaconBlockHeader, network::Network, signing_root, tree_hash::TreeHash,
    DOMAIN_SYNC_COMMITTEE,
};
use sails_rs::scale_codec::{Decode, DecodeAll, Encode};

fn decode_exact<T: Decode>(bytes: &[u8]) -> Result<T, BackendError> {
    T::decode_all(&mut &bytes[..]).map_err(|_| BackendError::MalformedResponse)
}

async fn call<B: BlsBackend>(backend: &B, request: Request) -> Result<Response, BackendError> {
    decode_exact(&backend.call(request).await?)
}

pub(crate) async fn verify_sync_committee_signature<B: BlsBackend>(
    network: &Network,
    pub_keys: Vec<G1>,
    attested_header: &BeaconBlockHeader,
    signature: &G2,
    signature_slot: u64,
    backend: &B,
) -> Result<bool, BackendError> {
    let signature = G2Affine::from(*signature);
    if signature.check().is_err() {
        return Ok(false);
    }
    let domain = signing_root::compute_domain(
        DOMAIN_SYNC_COMMITTEE,
        network.fork_version(signature_slot.saturating_sub(1)),
        network.genesis_validators_root(),
    );
    let signing_root = signing_root::compute(attested_header.tree_hash_root(), domain);

    let points: ArkScale<Vec<G1>> = pub_keys.into();
    let response = call(
        backend,
        Request::AggregateG1 {
            points: points.encode(),
        },
    )
    .await?;
    let Response::AggregateG1(bytes) = response else {
        return Err(BackendError::MalformedResponse);
    };
    let pub_key_aggregated = decode_exact::<ArkScale<G1>>(&bytes)?;
    // Ensure AggregatePublicKey is not at infinity.
    if pub_key_aggregated.0.is_zero() {
        return Ok(false);
    }
    let pub_key = G1Affine::from(pub_key_aggregated.0);
    if !pub_key.is_on_curve() {
        return Err(BackendError::MalformedResponse);
    }

    let response = call(
        backend,
        Request::MapToG2Affine {
            message: signing_root.as_ref().to_vec(),
        },
    )
    .await?;
    let Response::MapToG2Affine(bytes) = response else {
        return Err(BackendError::MalformedResponse);
    };
    let message = decode_exact::<ArkScale<G2Affine>>(&bytes)?;
    if message.0.check().is_err() || message.0.is_zero() {
        return Err(BackendError::MalformedResponse);
    }

    let generator_g1_negative = G1Affine::generator().neg();
    let a: ArkScale<Vec<G1Affine>> = vec![generator_g1_negative, pub_key].into();
    let b: ArkScale<Vec<G2Affine>> = vec![signature, message.0].into();
    let response = call(
        backend,
        Request::MultiMillerLoop {
            a: a.encode(),
            b: b.encode(),
        },
    )
    .await?;
    let Response::MultiMillerLoop(miller_loop) = response else {
        return Err(BackendError::MalformedResponse);
    };
    // This result is forwarded to the builtin, but its bytes still must be exact.
    decode_exact::<ArkScale<<Bls12_381 as Pairing>::TargetField>>(&miller_loop)?;
    let response = call(backend, Request::FinalExponentiation { f: miller_loop }).await?;
    let Response::FinalExponentiation(bytes) = response else {
        return Err(BackendError::MalformedResponse);
    };
    let exp = decode_exact::<ArkScale<<Bls12_381 as Pairing>::TargetField>>(&bytes)?;
    Ok(<Bls12_381 as Pairing>::TargetField::ONE == exp.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_response_decoding_is_exact_at_both_layers() {
        let point: ArkScale<G1> = G1Affine::generator().into_group().into();
        let payload = point.encode();
        let mut response = Response::AggregateG1(payload.clone()).encode();
        assert!(matches!(
            decode_exact::<Response>(&response),
            Ok(Response::AggregateG1(_))
        ));
        response.push(0);
        assert_eq!(
            decode_exact::<Response>(&response),
            Err(BackendError::MalformedResponse)
        );
        assert!(decode_exact::<ArkScale<G1>>(&payload).is_ok());
        let mut trailing_payload = payload;
        trailing_payload.push(0);
        assert!(matches!(
            decode_exact::<ArkScale<G1>>(&trailing_payload),
            Err(BackendError::MalformedResponse)
        ));
        assert!(matches!(
            decode_exact::<ArkScale<G1>>(&[]),
            Err(BackendError::MalformedResponse)
        ));
    }
}
