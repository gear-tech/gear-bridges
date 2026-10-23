//! Environment-independent consensus verification shared by the actor and native clients.
//!
//! Callers supply an immutable state snapshot and time, then commit the owned result
//! only if that snapshot is still current. Backend failures are not protocol verdicts.

use crate::{BeaconBlockHeader, Error, Init, Network, Slot, SyncCommitteeKeys, Update, G1};
use alloc::{boxed::Box, vec::Vec};
use ark_ec::CurveGroup;
use ark_serialize::CanonicalSerialize;
use core::{fmt, future::Future};
use ethereum_common::{
    base_types::{Bitvector, BytesFixed, FixedArray},
    beacon::{BLSPubKey, SyncAggregate, SyncCommittee},
    merkle,
    tree_hash::TreeHash,
    utils as eth_utils, SYNC_COMMITTEE_SIZE,
};
use sails_rs::scale_codec::DecodeAll;

pub use gbuiltin_bls381::{Request, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    Unavailable,
    MalformedResponse,
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BLS backend {self:?}")
    }
}

impl core::error::Error for BackendError {}

#[derive(Debug)]
pub enum VerificationError {
    Protocol(Error),
    Backend(BackendError),
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "Beacon verification: {error:?}"),
            Self::Backend(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl core::error::Error for VerificationError {}

impl From<Error> for VerificationError {
    fn from(error: Error) -> Self {
        Self::Protocol(error)
    }
}

impl From<BackendError> for VerificationError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}

/// The only environment seam. Return the complete SCALE-encoded builtin response;
/// the verifier checks its variant and consumes both outer and inner bytes exactly.
pub trait BlsBackend: Sync {
    fn call(&self, request: Request) -> impl Future<Output = Result<Vec<u8>, BackendError>> + Send;
}

#[derive(Clone, Copy)]
pub struct VerificationState<'a> {
    pub network: &'a Network,
    pub finalized_slot: Slot,
    pub current_committee: &'a SyncCommitteeKeys,
    pub next_committee: &'a SyncCommitteeKeys,
}

#[derive(Debug)]
pub struct VerifiedBootstrap {
    pub network: Network,
    pub finalized_header: BeaconBlockHeader,
    pub current_committee: Box<SyncCommitteeKeys>,
    pub next_committee: Box<SyncCommitteeKeys>,
}

#[derive(Debug)]
pub struct VerifiedUpdate {
    pub finalized_header: Option<BeaconBlockHeader>,
    /// When present, rotate the stored next committee to current and install this
    /// committee as next, atomically with the finalized header.
    pub next_committee: Option<Box<SyncCommitteeKeys>>,
}

pub async fn verify_bootstrap<B: BlsBackend>(
    init: Init,
    now_ms: u64,
    backend: &B,
) -> Result<VerifiedBootstrap, VerificationError> {
    if init.bootstrap_header.tree_hash_root() != init.trusted_bootstrap_root {
        return Err(Error::InvalidBootstrapRoot.into());
    }
    let period = eth_utils::calculate_period(init.update.finalized_header.slot)
        .checked_sub(1)
        .ok_or(Error::UnsupportedBootstrapPeriod)?;
    let Init {
        network,
        bootstrap_header,
        sync_committee_current_pub_keys,
        sync_committee_current_aggregate_pubkey,
        sync_committee_current_branch,
        update,
        sync_aggregate_encoded,
        trusted_bootstrap_root: _,
    } = init;
    let sync_aggregate = SyncAggregate::decode_all(&mut &sync_aggregate_encoded[..])
        .map_err(|_| Error::InvalidSyncAggregate)?;
    let sync_committee_current = construct_sync_committee(
        sync_committee_current_aggregate_pubkey,
        &sync_committee_current_pub_keys,
    )
    .ok_or(Error::InvalidPublicKeys)?;

    if !(bootstrap_header.slot <= update.finalized_header.slot
        && eth_utils::calculate_period(bootstrap_header.slot)
            == eth_utils::calculate_period(update.finalized_header.slot))
    {
        return Err(Error::InvalidBootstrapUpdate.into());
    }
    if !merkle::is_current_committee_proof_valid(
        &network,
        &bootstrap_header,
        &sync_committee_current,
        &sync_committee_current_branch,
    ) {
        return Err(Error::InvalidBootstrapProof.into());
    }

    let verified = verify_decoded_update(
        VerificationState {
            network: &network,
            finalized_slot: eth_utils::calculate_slot(period),
            current_committee: &sync_committee_current_pub_keys,
            next_committee: &sync_committee_current_pub_keys,
        },
        now_ms,
        update,
        sync_aggregate,
        backend,
    )
    .await?;
    match (verified.finalized_header, verified.next_committee) {
        (Some(finalized_header), Some(next_committee)) => Ok(VerifiedBootstrap {
            network,
            finalized_header,
            current_committee: sync_committee_current_pub_keys,
            next_committee,
        }),
        _ => Err(Error::InvalidBootstrapUpdate.into()),
    }
}

pub async fn verify_update<B: BlsBackend>(
    state: VerificationState<'_>,
    now_ms: u64,
    update: Update,
    encoded_aggregate: &[u8],
    backend: &B,
) -> Result<VerifiedUpdate, VerificationError> {
    let aggregate = SyncAggregate::decode_all(&mut &encoded_aggregate[..])
        .map_err(|_| Error::InvalidSyncAggregate)?;
    verify_decoded_update(state, now_ms, update, aggregate, backend).await
}

async fn verify_decoded_update<B: BlsBackend>(
    state: VerificationState<'_>,
    now_ms: u64,
    update: Update,
    aggregate: SyncAggregate,
    backend: &B,
) -> Result<VerifiedUpdate, VerificationError> {
    let Update {
        signature_slot,
        attested_header,
        finalized_header,
        sync_committee_signature,
        sync_committee_next_aggregate_pubkey,
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
        finality_branch,
    } = update;
    let network = state.network;
    let update_slot_finalized = finalized_header.slot;
    let current_slot = network
        .current_slot(now_ms)
        .ok_or(Error::InvalidTimestamp)?;
    if !(current_slot >= signature_slot
        && signature_slot > attested_header.slot
        && attested_header.slot >= update_slot_finalized)
    {
        return Err(Error::InvalidTimestamp.into());
    }
    let store_period = eth_utils::calculate_period(state.finalized_slot);
    let update_sig_period = eth_utils::calculate_period(signature_slot);
    if eth_utils::calculate_period(update_slot_finalized) > store_period.saturating_add(1) {
        return Err(Error::InvalidPeriod.into());
    }
    if eth_utils::calculate_epoch(attested_header.slot) < network.epoch_deneb() {
        return Err(Error::InvalidPeriod.into());
    }
    let sync_committee = if update_sig_period == store_period + 1 {
        state.next_committee
    } else if update_sig_period == store_period {
        state.current_committee
    } else {
        return Err(Error::InvalidPeriod.into());
    };
    let pub_keys = get_participating_keys(sync_committee, &aggregate.sync_committee_bits);
    if pub_keys.len() * 3 < SYNC_COMMITTEE_SIZE * 2 {
        return Err(Error::LowVoteCount.into());
    }
    if !crate::crypto::verify_sync_committee_signature(
        network,
        pub_keys,
        &attested_header,
        &sync_committee_signature.0 .0,
        signature_slot,
        backend,
    )
    .await?
    {
        return Err(Error::InvalidSignature.into());
    }

    let mut finalized_header_update = None;
    if update_slot_finalized > state.finalized_slot {
        if merkle::is_finality_proof_valid(
            network,
            &attested_header,
            &finalized_header,
            &finality_branch,
        ) {
            finalized_header_update = Some(finalized_header);
        } else {
            return Err(Error::InvalidFinalityProof.into());
        }
    }
    let committee_update = verify_next_committee(
        network,
        store_period,
        &attested_header,
        update_slot_finalized,
        sync_committee_next_aggregate_pubkey,
        sync_committee_next_pub_keys,
        sync_committee_next_branch,
    )?;
    // The stored keys always describe the finalized period and its successor.
    if eth_utils::calculate_period(update_slot_finalized) > store_period
        && committee_update.is_none()
    {
        return Err(Error::InvalidNextSyncCommitteeProof.into());
    }
    if finalized_header_update.is_none() && committee_update.is_none() {
        return Err(Error::NotActual.into());
    }
    Ok(VerifiedUpdate {
        finalized_header: finalized_header_update,
        next_committee: committee_update,
    })
}

fn verify_next_committee(
    network: &Network,
    store_period: u64,
    attested_header: &BeaconBlockHeader,
    update_slot_finalized: u64,
    aggregate_pubkey: Option<BLSPubKey>,
    keys: Option<Box<SyncCommitteeKeys>>,
    branch: Option<Vec<[u8; 32]>>,
) -> Result<Option<Box<SyncCommitteeKeys>>, Error> {
    let update_period_finalized = eth_utils::calculate_period(update_slot_finalized);
    let (true, Some(aggregate_pubkey), Some(keys), Some(branch)) = (
        eth_utils::calculate_period(attested_header.slot) == update_period_finalized,
        aggregate_pubkey,
        keys,
        branch,
    ) else {
        return Ok(None);
    };
    let committee =
        construct_sync_committee(aggregate_pubkey, &keys).ok_or(Error::InvalidPublicKeys)?;
    if !merkle::is_next_committee_proof_valid(network, attested_header, &committee, &branch) {
        return Err(Error::InvalidNextSyncCommitteeProof);
    }
    if update_period_finalized == store_period + 1 {
        return Ok(Some(keys));
    }
    Ok(None)
}

fn construct_sync_committee(
    aggregate_pubkey: BLSPubKey,
    public_keys: &SyncCommitteeKeys,
) -> Option<SyncCommittee> {
    let mut pub_keys = Vec::with_capacity(SYNC_COMMITTEE_SIZE);
    for pub_key in public_keys.0.iter() {
        let pub_key = pub_key.0 .0.into_affine();
        // The authenticated committee root binds the unique compressed point.
        // Reject omitted-coordinate malleability before authenticating that root.
        if !pub_key.is_on_curve() {
            return None;
        }
        let mut buffer = BytesFixed(FixedArray([0u8; 48]));
        pub_key.serialize_compressed(buffer.0 .0.as_mut()).ok()?;
        pub_keys.push(buffer);
    }
    Some(SyncCommittee {
        pubkeys: FixedArray(pub_keys.try_into().ok()?),
        aggregate_pubkey,
    })
}

fn get_participating_keys(
    committee: &SyncCommitteeKeys,
    bitfield: &Bitvector<SYNC_COMMITTEE_SIZE>,
) -> Vec<G1> {
    bitfield
        .iter()
        .zip(committee.0.iter())
        .filter_map(|(bit, pub_key)| bit.then_some(pub_key.0 .0))
        .collect()
}
