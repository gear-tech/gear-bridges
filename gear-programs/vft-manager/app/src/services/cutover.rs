use super::{
    decode_complete_reply, Config, Error, NativeWrapperConfig, NativeWrapperMode, TokenSupply,
    VftManager,
};
use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
use sails_rs::{gstd::msg, prelude::*};
use vft_client::{
    vft::io::{BalanceOf, Transfer},
    vft_admin::io::IsPaused,
};

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
    pub generation: u64,
    pub predecessor_generation: Option<u64>,
    pub custody_started: bool,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct CutoverAbort {
    pub generation: u64,
    pub successor: ActorId,
    pub last_receipt_slot: u64,
    pub config_fingerprint: H256,
}

fn same_generation(service: &VftManager, generation: u64) -> bool {
    service.state().cutover.generation == generation
}

pub(super) fn abort(service: &mut VftManager, expected_generation: u64) {
    assert!(service.state().is_paused, "Not paused");
    let intent = service.state().cutover.clone();
    assert_eq!(
        intent.generation, expected_generation,
        "Cutover generation changed"
    );
    assert_eq!(intent.phase, CutoverPhase::Draining, "Source not draining");
    assert!(!intent.custody_started, "Custody child already dispatched");
    let state = service.state_mut();
    state.cutover_aborts.insert(
        intent.generation,
        CutoverAbort {
            generation: intent.generation,
            successor: intent.successor.expect("Missing successor"),
            last_receipt_slot: intent.last_receipt_slot.expect("Missing boundary"),
            config_fingerprint: intent.config_fingerprint.expect("Missing fingerprint"),
        },
    );
    state.cutover = state.cutover_prior.take().expect("Missing prior lifecycle");
    state.cutover.generation = intent
        .generation
        .checked_add(1)
        .expect("Generation overflow");
    state.frozen_mappings.clear();
    state.custody_completed.clear();
    state.custody_latest.clear();
    state.custody_preflight = 0;
}

pub(super) async fn restore_aborted(service: &mut VftManager, expected_generation: u64) {
    assert!(service.state().is_paused, "Not paused");
    let intent = service.state().cutover.clone();
    assert_eq!(
        intent.generation, expected_generation,
        "Cutover generation changed"
    );
    assert_eq!(
        intent.phase,
        CutoverPhase::AwaitingSource,
        "Destination not prepared"
    );
    let generation = intent
        .predecessor_generation
        .expect("Missing source generation");
    let payload = ("VftManager", "CutoverAbort", generation).encode();
    let proof: Option<CutoverAbort> = query_payload(
        intent.predecessor.expect("Missing predecessor"),
        payload,
        ("VftManager", "CutoverAbort").encode(),
        service.config(),
    )
    .await;
    if !same_generation(service, intent.generation) {
        return;
    }
    assert!(service.state().is_paused, "Not paused");
    assert!(
        proof
            == Some(CutoverAbort {
                generation,
                successor: Syscall::program_id(),
                last_receipt_slot: intent.minimum_receipt_slot.checked_sub(1).unwrap(),
                config_fingerprint: intent.config_fingerprint.unwrap(),
            }),
        "Source abort does not match preparation"
    );
    service.ensure_quiescent();
    let state = service.state_mut();
    state.cutover = state.cutover_prior.take().expect("Missing prior lifecycle");
    state.cutover.generation = intent
        .generation
        .checked_add(1)
        .expect("Generation overflow");
    state.frozen_mappings.clear();
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
    result.in_flight_transfer_count = state.custody_active;
    result
}

fn fingerprint(service: &VftManager) -> (H256, Vec<(ActorId, H160, TokenSupply)>) {
    let state = service.state();
    let mut mappings = state.token_map.read_state();
    mappings.sort_unstable_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    let mut eth = collections::BTreeSet::new();
    let mut previous = None;
    for &(token, address, _) in &mappings {
        assert!(previous != Some(token), "Duplicate Gear identity");
        assert!(eth.insert(address), "Duplicate Ethereum identity");
        previous = Some(token);
    }
    let encoded = match state.native_wrapper {
        Some(wrapper) if wrapper.mode == NativeWrapperMode::HeldEscrowV2 => (
            *b"vft-manager/cutover-config/v2",
            state.gear_bridge_builtin,
            state.historical_proxy_address,
            state.erc20_manager_address,
            state.native_wrapper,
            service.config(),
            &mappings,
        )
            .encode(),
        _ => (
            *b"vft-manager/cutover-config/v1",
            state.gear_bridge_builtin,
            state.historical_proxy_address,
            state.erc20_manager_address,
            state.native_wrapper.map(|wrapper| wrapper.actor_id),
            service.config(),
            &mappings,
        )
            .encode(),
    };
    let hash = KeccakHasher::hash(&encoded);
    (hash, mappings)
}

pub(super) fn begin(
    service: &mut VftManager,
    destination: ActorId,
    boundary: u64,
    expected_generation: u64,
) {
    assert!(
        !destination.is_zero() && destination != Syscall::program_id(),
        "Invalid successor"
    );
    boundary.checked_add(1).expect("Receipt boundary overflow");
    let state = service.state();
    assert_eq!(
        state.cutover.generation, expected_generation,
        "Cutover generation changed"
    );
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
    state.cutover_prior = Some(state.cutover.clone());
    state.cutover.generation = state
        .cutover
        .generation
        .checked_add(1)
        .expect("Generation overflow");
    state.cutover.custody_started = false;
    state.custody_preflight = 0;
    state.custody_cursor = 0;
    state.custody_final_cursor = 0;
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
pub(super) async fn query<T: Decode>(
    target: ActorId,
    service: &str,
    route: &str,
    config: &Config,
) -> T {
    let payload = (service, route).encode();
    query_payload(target, payload.clone(), payload, config).await
}

async fn query_payload<T: Decode>(
    target: ActorId,
    payload: Vec<u8>,
    route: Vec<u8>,
    config: &Config,
) -> T {
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
    assert!(bytes.starts_with(&route), "Unexpected cutover query route");
    let mut input = &bytes[route.len()..];
    let result = T::decode(&mut input).expect("Invalid cutover query reply");
    assert!(input.is_empty(), "Trailing cutover query bytes");
    result
}

fn ensure_unused(service: &VftManager, floor: u64) {
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
        service.state().cutover.predecessor.is_none(),
        "Destination has an existing predecessor"
    );
    assert_eq!(
        service.state().cutover.minimum_receipt_slot,
        floor,
        "Different constructor receipt floor"
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
        assert!(
            service.state().cutover.predecessor == Some(source),
            "Different predecessor"
        );
        assert_eq!(
            service.state().cutover.minimum_receipt_slot,
            floor,
            "Different boundary"
        );
        return;
    }
    ensure_unused(service, floor);
    let revision = service.state().economic_revision;
    let generation = service.state().cutover.generation;
    let (hash, mappings) = fingerprint(service);
    let config = service.config().clone();
    let source_status: CutoverStatus = query(source, "VftManager", "CutoverStatus", &config).await;
    if !same_generation(service, generation) {
        return;
    }
    assert_eq!(
        source_status.phase,
        CutoverPhase::Draining,
        "Source not draining"
    );
    assert!(
        source_status.successor == Some(Syscall::program_id()),
        "Source names another successor"
    );
    assert_eq!(
        source_status.last_receipt_slot,
        Some(boundary),
        "Different boundary"
    );
    assert!(
        source_status.config_fingerprint == Some(hash),
        "Different economic configuration"
    );
    assert_eq!(
        source_status.transfer_count as usize,
        mappings.len(),
        "Different token inventory"
    );
    ensure_unused(service, floor);
    assert_eq!(
        service.state().economic_revision,
        revision,
        "Configuration changed during preparation"
    );
    let state = service.state_mut();
    state.cutover_prior = Some(state.cutover.clone());
    state.cutover.generation = generation.checked_add(1).expect("Generation overflow");
    state.cutover.predecessor_generation = Some(source_status.generation);
    state.frozen_mappings = mappings;
    state.cutover.predecessor = Some(source);
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
    assert!(
        state.cutover.successor == Some(destination),
        "Different successor"
    );
}

fn validate_prepared(service: &VftManager, destination: &CutoverStatus) {
    let source = &service.state().cutover;
    assert_eq!(
        destination.predecessor_generation,
        Some(source.generation),
        "Different source generation"
    );
    assert_eq!(
        destination.phase,
        CutoverPhase::AwaitingSource,
        "Destination not prepared"
    );
    assert!(
        destination.predecessor == Some(Syscall::program_id()),
        "Different predecessor"
    );
    assert_eq!(
        destination.minimum_receipt_slot,
        source.last_receipt_slot.unwrap().checked_add(1).unwrap(),
        "Different destination floor"
    );
    assert!(
        destination.config_fingerprint == source.config_fingerprint,
        "Different fingerprint"
    );
    assert!(
        destination.successor.is_none() && destination.last_receipt_slot.is_none(),
        "Destination already draining"
    );
}

async fn token_query<Action: sails_rs::calls::ActionIo>(
    token: ActorId,
    params: &Action::Params,
    config: &Config,
) -> Option<Action::Reply>
where
    Action::Reply: Encode,
{
    let bytes = msg::send_bytes_with_gas_for_reply(
        token,
        Action::encode_call(params),
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .ok()?
    .up_to(Some(config.reply_timeout))
    .ok()?
    .await
    .ok()?;
    decode_complete_reply::<Action>(&bytes, |_| Error::InvalidReply).ok()
}

async fn balance(token: ActorId, config: &Config) -> Option<U256> {
    token_query::<BalanceOf>(token, &Syscall::program_id(), config).await
}

async fn blocking_native_redemptions(wrapper: NativeWrapperConfig, config: &Config) -> u64 {
    let method = match wrapper.mode {
        NativeWrapperMode::EscrowV1 => "OutstandingRedemptions",
        NativeWrapperMode::HeldEscrowV2 => "BlockingRedemptions",
    };
    query(wrapper.actor_id, "NativeEscrow", method, config).await
}

pub(super) async fn upgrade(service: &mut VftManager, destination: ActorId) {
    if service.state().cutover.phase == CutoverPhase::Retired {
        assert!(
            service.state().cutover.successor == Some(destination),
            "Different successor"
        );
        return;
    }
    ensure_transfer_state(service, destination);
    let generation = service.state().cutover.generation;
    let config = service.config().clone();
    let prepared: CutoverStatus = query(destination, "VftManager", "CutoverStatus", &config).await;
    if !same_generation(service, generation) {
        return;
    }
    let paused: bool = query(destination, "VftManager", "IsPaused", &config).await;
    if !same_generation(service, generation) {
        return;
    }
    assert!(paused, "Destination unpaused");
    ensure_transfer_state(service, destination);
    validate_prepared(service, &prepared);
    if service.state().cutover.phase == CutoverPhase::Draining {
        if let Some(wrapper) = service.state().native_wrapper {
            let manager: Option<ActorId> =
                query(wrapper.actor_id, "NativeEscrow", "Manager", &config).await;
            if !same_generation(service, generation) {
                return;
            }
            let blocking = blocking_native_redemptions(wrapper, &config).await;
            if !same_generation(service, generation) {
                return;
            }
            assert!(
                manager == Some(Syscall::program_id()),
                "Unexpected wrapper manager"
            );
            assert_eq!(blocking, 0, "Blocking native redemptions");
        }
        ensure_transfer_state(service, destination);
        // At most 16 frozen tokens per invocation; query failure stays abortable.
        let end =
            (service.state().custody_preflight + 16).min(service.state().frozen_mappings.len());
        while service.state().custody_preflight < end {
            let position = service.state().custody_preflight;
            let token = service.state().frozen_mappings[position].0;
            let observed = balance(token, &config).await;
            if !same_generation(service, generation) {
                return;
            }
            ensure_transfer_state(service, destination);
            if observed.is_none() {
                return;
            }
            let paused = token_query::<IsPaused>(token, &(), &config).await;
            if !same_generation(service, generation) {
                return;
            }
            ensure_transfer_state(service, destination);
            if paused != Some(false) || service.state().custody_preflight != position {
                return;
            }
            service.state_mut().custody_preflight += 1;
        }
        if service.state().custody_preflight != service.state().frozen_mappings.len() {
            return;
        }
    }
    // Index the immutable frozen list instead of sorting/hashing on each transfer.
    let count = service.state().frozen_mappings.len();
    for _ in 0..count.min(16) {
        let position = service.state().custody_cursor % count;
        service.state_mut().custody_cursor = (position + 1) % count;
        let token = service.state().frozen_mappings[position].0;
        if unresolved(service, token) {
            continue;
        }
        let amount = balance(token, &config).await;
        if !same_generation(service, generation) {
            return;
        }
        let Some(amount) = amount else {
            return;
        };
        ensure_transfer_state(service, destination);
        // Another Upgrade may have claimed this token while its balance query awaited.
        if unresolved(service, token) {
            continue;
        }
        if amount.is_zero() {
            service.state_mut().custody_completed.insert(token);
            continue;
        }
        service.state_mut().cutover.phase = CutoverPhase::Transferring;
        service.state_mut().cutover.custody_started = true;
        service.state_mut().custody_final_cursor = 0;
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
        state.custody_active = state
            .custody_active
            .checked_add(1)
            .expect("Custody count overflow");
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
        if !same_generation(service, generation) {
            return;
        }
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
    let end = (service.state().custody_final_cursor + 16).min(count);
    while service.state().custody_final_cursor < end {
        let position = service.state().custody_final_cursor;
        let token = service.state().frozen_mappings[position].0;
        let remaining = balance(token, &config).await;
        if !same_generation(service, generation) {
            return;
        }
        let Some(remaining) = remaining else {
            return;
        };
        ensure_transfer_state(service, destination);
        if !remaining.is_zero() {
            service.state_mut().custody_completed.remove(&token);
            service.state_mut().custody_final_cursor = 0;
            return;
        }
        // A concurrent continuation may have sent another child while this query awaited.
        if service.state().custody_completed.len() != service.state().frozen_mappings.len()
            || status(service).in_flight_transfer_count != 0
        {
            return;
        }
        if service.state().custody_final_cursor != position {
            return;
        }
        service.state_mut().custody_final_cursor += 1;
    }
    if service.state().custody_final_cursor != count {
        return;
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
            decode_complete_reply::<Transfer>(&bytes, |_| Error::InvalidReply).ok()
        }) {
            Some(true) => CustodyTransferOutcome::Completed,
            Some(false) => CustodyTransferOutcome::Rejected,
            None => CustodyTransferOutcome::Ambiguous,
        },
        _ => CustodyTransferOutcome::Ambiguous,
    };
    let token = attempt.token;
    service.state_mut().custody_transfers[index].outcome = outcome;
    if matches!(
        outcome,
        CustodyTransferOutcome::Completed | CustodyTransferOutcome::Rejected
    ) {
        let state = service.state_mut();
        state.custody_active = state
            .custody_active
            .checked_sub(1)
            .expect("Custody count underflow");
    }
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
    if !same_generation(service, intent.generation) {
        return;
    }
    assert_eq!(
        retired.generation,
        intent.predecessor_generation.unwrap(),
        "Different source generation"
    );
    assert_eq!(retired.phase, CutoverPhase::Retired, "Source not retired");
    assert!(
        retired.successor == Some(Syscall::program_id()),
        "Different successor"
    );
    assert_eq!(
        retired
            .last_receipt_slot
            .and_then(|slot| slot.checked_add(1)),
        Some(intent.minimum_receipt_slot),
        "Different boundary"
    );
    assert!(
        retired.config_fingerprint == intent.config_fingerprint,
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
        let manager: Option<ActorId> =
            query(wrapper.actor_id, "NativeEscrow", "Manager", &config).await;
        if !same_generation(service, intent.generation) {
            return;
        }
        let burner: ActorId = query(wrapper.actor_id, "VftAdmin", "Burner", &config).await;
        if !same_generation(service, intent.generation) {
            return;
        }
        let blocking = blocking_native_redemptions(wrapper, &config).await;
        if !same_generation(service, intent.generation) {
            return;
        }
        assert!(
            manager == Some(Syscall::program_id()),
            "Wrapper manager not handed off"
        );
        assert!(
            burner == Syscall::program_id(),
            "Wrapper burner not handed off"
        );
        assert_eq!(blocking, 0, "Blocking native redemptions");
    }
    service.ensure_quiescent();
    assert!(
        service.state().cutover == intent,
        "Cutover changed during finalization"
    );
    service.state_mut().cutover.phase = CutoverPhase::Ready;
}
