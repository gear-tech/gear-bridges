use super::{Config, TokenSupply, VftManager};
use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
use sails_rs::{calls::ActionIo, gstd::msg, prelude::*};
use vft_client::vft::io::{BalanceOf, Transfer};

#[derive(Clone, Copy, Debug, Default, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum CutoverPhase {
    #[default]
    Active,
    Draining,
    AwaitingSource,
    Transferring,
    Retired,
    Ready,
}

#[derive(Clone, Debug, Default, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct CutoverStatus {
    pub phase: CutoverPhase,
    pub predecessor: Option<ActorId>,
    pub successor: Option<ActorId>,
    pub minimum_receipt_slot: u64,
    pub last_receipt_slot: Option<u64>,
    pub config_fingerprint: Option<H256>,
    pub transfer_count: u32,
    pub completed_transfer_count: u32,
    pub in_flight_transfer_count: u32,
}

#[derive(Clone, Copy, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum CustodyTransferOutcome {
    InFlight,
    Completed,
    Rejected,
    Ambiguous,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct CustodyTransferAttempt {
    pub token: ActorId,
    pub amount: U256,
    pub child: MessageId,
    pub outcome: CustodyTransferOutcome,
}

pub(super) fn status(service: &VftManager) -> CutoverStatus {
    let state = service.state();
    let mut result = state.cutover.clone();
    result.completed_transfer_count = state
        .custody_completed
        .len()
        .try_into()
        .expect("Too many tokens");
    result.in_flight_transfer_count = state
        .custody_latest
        .values()
        .filter(|&&index| {
            matches!(
                state.custody_transfers[index].outcome,
                CustodyTransferOutcome::InFlight | CustodyTransferOutcome::Ambiguous
            )
        })
        .count()
        .try_into()
        .expect("Too many transfers");
    result
}

fn fingerprint(service: &VftManager) -> (H256, Vec<(ActorId, H160, TokenSupply)>) {
    let state = service.state();
    let mut mappings = state.token_map.read_state();
    mappings.sort_unstable_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    let mut eth = collections::BTreeSet::new();
    let mut previous = None;
    for &(token, address, _) in &mappings {
        assert_ne!(previous, Some(token), "Duplicate Gear identity");
        assert!(eth.insert(address), "Duplicate Ethereum identity");
        previous = Some(token);
    }
    let hash = KeccakHasher::hash(
        &(
            *b"vft-manager/cutover-config/v1",
            state.gear_bridge_builtin,
            state.historical_proxy_address,
            state.erc20_manager_address,
            state.native_wrapper,
            service.config(),
            &mappings,
        )
            .encode(),
    );
    (hash, mappings)
}

pub(super) fn begin(service: &mut VftManager, destination: ActorId, boundary: u64) {
    assert!(
        !destination.is_zero() && destination != Syscall::program_id(),
        "Invalid successor"
    );
    boundary.checked_add(1).expect("Receipt boundary overflow");
    let state = service.state();
    if state.cutover.phase != CutoverPhase::Active {
        assert!(
            matches!(
                state.cutover.phase,
                CutoverPhase::Draining | CutoverPhase::Transferring | CutoverPhase::Retired
            ) && state.cutover.successor == Some(destination)
                && state.cutover.last_receipt_slot == Some(boundary),
            "Different cutover intent"
        );
        return;
    }
    assert!(
        boundary >= state.cutover.minimum_receipt_slot,
        "Boundary below deployment floor"
    );
    assert!(
        state
            .maximum_admitted_receipt_slot
            .is_none_or(|slot| slot <= boundary),
        "Boundary excludes an admitted receipt"
    );
    let (hash, mappings) = fingerprint(service);
    let count = mappings.len().try_into().expect("Too many tokens");
    let state = service.state_mut();
    state.frozen_mappings = mappings;
    state.custody_completed.clear();
    state.custody_latest.clear();
    state.cutover.phase = CutoverPhase::Draining;
    state.cutover.successor = Some(destination);
    state.cutover.last_receipt_slot = Some(boundary);
    state.cutover.config_fingerprint = Some(hash);
    state.cutover.transfer_count = count;
}

// Authenticate exact Sails 0.9.2 service/route framing for bounded cross-program queries.
async fn query<T: Decode>(target: ActorId, service: &str, route: &str, config: &Config) -> T {
    let payload = (service, route).encode();
    let bytes = msg::send_bytes_with_gas_for_reply(
        target,
        &payload,
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .expect("Query dispatch failed")
    .up_to(Some(config.reply_timeout))
    .expect("Query timeout installation failed")
    .await
    .expect("Cutover query failed");
    assert!(
        bytes.starts_with(&payload),
        "Unexpected cutover query route"
    );
    let mut input = &bytes[payload.len()..];
    let result = T::decode(&mut input).expect("Invalid cutover query reply");
    assert!(input.is_empty(), "Trailing cutover query bytes");
    result
}

fn ensure_unused(service: &VftManager) {
    service.ensure_quiescent();
    assert!(
        !service.state().has_economic_activity,
        "Destination has economic activity"
    );
    assert_eq!(
        service.state().cutover.phase,
        CutoverPhase::Active,
        "Destination already prepared"
    );
    assert!(
        service.state().cutover.predecessor.is_none()
            && service.state().cutover.minimum_receipt_slot == 0,
        "Destination has an existing deployment domain"
    );
}

pub(super) async fn prepare(service: &mut VftManager, source: ActorId, boundary: u64) {
    assert!(
        !source.is_zero() && source != Syscall::program_id(),
        "Invalid predecessor"
    );
    let floor = boundary.checked_add(1).expect("Receipt boundary overflow");
    if service.state().cutover.phase == CutoverPhase::AwaitingSource {
        service.ensure_quiescent();
        assert_eq!(
            service.state().cutover.predecessor,
            Some(source),
            "Different predecessor"
        );
        assert_eq!(
            service.state().cutover.minimum_receipt_slot,
            floor,
            "Different boundary"
        );
        return;
    }
    ensure_unused(service);
    let revision = service.state().economic_revision;
    let (hash, mappings) = fingerprint(service);
    let config = service.config().clone();
    let source_status: CutoverStatus = query(source, "VftManager", "CutoverStatus", &config).await;
    assert_eq!(
        source_status.phase,
        CutoverPhase::Draining,
        "Source not draining"
    );
    assert_eq!(
        source_status.successor,
        Some(Syscall::program_id()),
        "Source names another successor"
    );
    assert_eq!(
        source_status.last_receipt_slot,
        Some(boundary),
        "Different boundary"
    );
    assert_eq!(
        source_status.config_fingerprint,
        Some(hash),
        "Different economic configuration"
    );
    assert_eq!(
        source_status.transfer_count as usize,
        mappings.len(),
        "Different token inventory"
    );
    ensure_unused(service);
    assert_eq!(
        service.state().economic_revision,
        revision,
        "Configuration changed during preparation"
    );
    let state = service.state_mut();
    state.frozen_mappings = mappings;
    state.cutover.predecessor = Some(source);
    state.cutover.minimum_receipt_slot = floor;
    state.cutover.config_fingerprint = Some(hash);
    state.cutover.phase = CutoverPhase::AwaitingSource;
}

fn ensure_transfer_state(service: &VftManager, destination: ActorId) {
    service.ensure_quiescent();
    let state = service.state();
    assert!(
        matches!(
            state.cutover.phase,
            CutoverPhase::Draining | CutoverPhase::Transferring
        ),
        "Source not transferring"
    );
    assert_eq!(
        state.cutover.successor,
        Some(destination),
        "Different successor"
    );
}

fn validate_prepared(service: &VftManager, destination: &CutoverStatus) {
    let source = &service.state().cutover;
    assert_eq!(
        destination.phase,
        CutoverPhase::AwaitingSource,
        "Destination not prepared"
    );
    assert_eq!(
        destination.predecessor,
        Some(Syscall::program_id()),
        "Different predecessor"
    );
    assert_eq!(
        destination.minimum_receipt_slot,
        source.last_receipt_slot.unwrap().checked_add(1).unwrap(),
        "Different destination floor"
    );
    assert_eq!(
        destination.config_fingerprint, source.config_fingerprint,
        "Different fingerprint"
    );
    assert!(
        destination.successor.is_none() && destination.last_receipt_slot.is_none(),
        "Destination already draining"
    );
}

async fn balance(token: ActorId, config: &Config) -> U256 {
    let bytes = msg::send_bytes_with_gas_for_reply(
        token,
        BalanceOf::encode_call(Syscall::program_id()),
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .expect("Balance query dispatch failed")
    .up_to(Some(config.reply_timeout))
    .expect("Balance query timeout installation failed")
    .await
    .expect("Balance query failed");
    let amount = BalanceOf::decode_reply(&bytes).expect("Invalid balance reply");
    assert_eq!(
        bytes.len(),
        BalanceOf::ROUTE.len() + amount.encoded_size(),
        "Trailing balance bytes"
    );
    amount
}

pub(super) async fn upgrade(service: &mut VftManager, destination: ActorId) {
    if service.state().cutover.phase == CutoverPhase::Retired {
        assert_eq!(
            service.state().cutover.successor,
            Some(destination),
            "Different successor"
        );
        return;
    }
    ensure_transfer_state(service, destination);
    let config = service.config().clone();
    let prepared: CutoverStatus = query(destination, "VftManager", "CutoverStatus", &config).await;
    let paused: bool = query(destination, "VftManager", "IsPaused", &config).await;
    assert!(paused, "Destination unpaused");
    ensure_transfer_state(service, destination);
    validate_prepared(service, &prepared);
    if service.state().cutover.phase == CutoverPhase::Draining {
        if let Some(wrapper) = service.state().native_wrapper {
            let manager: Option<ActorId> = query(wrapper, "NativeEscrow", "Manager", &config).await;
            let outstanding: u64 =
                query(wrapper, "NativeEscrow", "OutstandingRedemptions", &config).await;
            assert_eq!(
                manager,
                Some(Syscall::program_id()),
                "Unexpected wrapper manager"
            );
            assert_eq!(outstanding, 0, "Outstanding native redemptions");
        }
        ensure_transfer_state(service, destination);
        service.state_mut().cutover.phase = CutoverPhase::Transferring;
    }
    // Index the immutable frozen list instead of sorting/hashing on each transfer.
    for position in 0..service.state().frozen_mappings.len() {
        let token = service.state().frozen_mappings[position].0;
        if unresolved(service, token) {
            continue;
        }
        let amount = balance(token, &config).await;
        ensure_transfer_state(service, destination);
        // Another Upgrade may have claimed this token while its balance query awaited.
        if unresolved(service, token) {
            continue;
        }
        if amount.is_zero() {
            service.state_mut().custody_completed.insert(token);
            continue;
        }
        if service
            .state()
            .custody_latest
            .get(&token)
            .is_some_and(|&index| {
                service.state().custody_transfers[index].outcome
                    == CustodyTransferOutcome::Completed
            })
        {
            panic!("Completed token has nonzero remaining custody");
        }
        let reply = msg::send_bytes_with_gas_for_reply(
            token,
            Transfer::encode_call(destination, amount),
            config.gas_for_token_ops,
            0,
            config.gas_for_reply_deposit,
        )
        .expect("Custody dispatch failed: roll back enqueue");
        let child = reply.waiting_reply_to;
        let state = service.state_mut();
        state.custody_completed.remove(&token);
        let index = state.custody_transfers.len();
        state.custody_transfers.push(CustodyTransferAttempt {
            token,
            amount,
            child,
            outcome: CustodyTransferOutcome::InFlight,
        });
        state.custody_latest.insert(token, index);
        state.custody_children.insert(child, index);
        let _ = reply
            .up_to(Some(config.reply_timeout))
            .expect("Custody timeout installation failed")
            .handle_reply(handle_upgrade_reply)
            .expect("Custody reply hook installation failed")
            .await;
        if service.state().custody_transfers[index].outcome == CustodyTransferOutcome::InFlight {
            service.state_mut().custody_transfers[index].outcome =
                CustodyTransferOutcome::Ambiguous;
        }
        ensure_transfer_state(service, destination);
    }
    if service.state().custody_completed.len() != service.state().frozen_mappings.len()
        || status(service).in_flight_transfer_count != 0
    {
        return;
    }
    // Final zero balances are mandatory, even after an authenticated true reply.
    for position in 0..service.state().frozen_mappings.len() {
        let token = service.state().frozen_mappings[position].0;
        let remaining = balance(token, &config).await;
        ensure_transfer_state(service, destination);
        assert!(remaining.is_zero(), "Nonzero final custody balance");
    }
    ensure_transfer_state(service, destination);
    service.state_mut().cutover.phase = CutoverPhase::Retired;
}

fn unresolved(service: &VftManager, token: ActorId) -> bool {
    service
        .state()
        .custody_latest
        .get(&token)
        .is_some_and(|&index| {
            matches!(
                service.state().custody_transfers[index].outcome,
                CustodyTransferOutcome::InFlight | CustodyTransferOutcome::Ambiguous
            )
        })
}

pub fn handle_upgrade_reply() {
    let Ok(child) = msg::reply_to() else {
        return;
    };
    let mut service = VftManager::new();
    let Some(index) = service.state().custody_children.get(&child).copied() else {
        return;
    };
    let attempt = &service.state().custody_transfers[index];
    if attempt.token != msg::source()
        || !matches!(
            attempt.outcome,
            CustodyTransferOutcome::InFlight | CustodyTransferOutcome::Ambiguous
        )
    {
        return;
    }
    let outcome = match msg::reply_code() {
        Ok(ReplyCode::Error(_)) => CustodyTransferOutcome::Rejected,
        Ok(ReplyCode::Success(_)) => match msg::load_bytes().ok().and_then(|bytes| {
            Transfer::decode_reply(&bytes)
                .ok()
                .filter(|reply| bytes.len() == Transfer::ROUTE.len() + reply.encoded_size())
        }) {
            Some(true) => CustodyTransferOutcome::Completed,
            Some(false) => CustodyTransferOutcome::Rejected,
            None => CustodyTransferOutcome::Ambiguous,
        },
        _ => CustodyTransferOutcome::Ambiguous,
    };
    let token = attempt.token;
    service.state_mut().custody_transfers[index].outcome = outcome;
    if outcome == CustodyTransferOutcome::Completed {
        service.state_mut().custody_completed.insert(token);
    }
}

pub(super) async fn finalize(service: &mut VftManager) {
    service.ensure_quiescent();
    if service.state().cutover.phase == CutoverPhase::Ready {
        return;
    }
    assert_eq!(
        service.state().cutover.phase,
        CutoverPhase::AwaitingSource,
        "Destination not awaiting source"
    );
    let intent = service.state().cutover.clone();
    let source = intent.predecessor.expect("Missing predecessor");
    let config = service.config().clone();
    let retired: CutoverStatus = query(source, "VftManager", "CutoverStatus", &config).await;
    assert_eq!(retired.phase, CutoverPhase::Retired, "Source not retired");
    assert_eq!(
        retired.successor,
        Some(Syscall::program_id()),
        "Different successor"
    );
    assert_eq!(
        retired
            .last_receipt_slot
            .and_then(|slot| slot.checked_add(1)),
        Some(intent.minimum_receipt_slot),
        "Different boundary"
    );
    assert_eq!(
        retired.config_fingerprint, intent.config_fingerprint,
        "Different fingerprint"
    );
    assert_eq!(
        retired.transfer_count as usize,
        service.state().frozen_mappings.len(),
        "Different token inventory"
    );
    assert_eq!(
        retired.completed_transfer_count, retired.transfer_count,
        "Incomplete custody transfers"
    );
    assert_eq!(
        retired.in_flight_transfer_count, 0,
        "Unresolved custody transfers"
    );
    if let Some(wrapper) = service.state().native_wrapper {
        let manager: Option<ActorId> = query(wrapper, "NativeEscrow", "Manager", &config).await;
        let burner: ActorId = query(wrapper, "VftAdmin", "Burner", &config).await;
        let outstanding: u64 =
            query(wrapper, "NativeEscrow", "OutstandingRedemptions", &config).await;
        assert_eq!(
            manager,
            Some(Syscall::program_id()),
            "Wrapper manager not handed off"
        );
        assert_eq!(
            burner,
            Syscall::program_id(),
            "Wrapper burner not handed off"
        );
        assert_eq!(outstanding, 0, "Outstanding native redemptions");
    }
    service.ensure_quiescent();
    assert_eq!(
        service.state().cutover,
        intent,
        "Cutover changed during finalization"
    );
    service.state_mut().cutover.phase = CutoverPhase::Ready;
}
