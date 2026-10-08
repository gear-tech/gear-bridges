use super::{construct_init, decode_signature};
use ::gtest::{CoreLog, Program, System, WasmProgram};
use ark_ec::Group;
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_client::{
    checkpoint_light_client_factory::io as factory_io, service_checkpoint_for::io as checkpoint_io,
    service_replay_back::io as replay_io, service_state::io as state_io,
    service_sync_update::io as sync_io, Order, StateData,
};
use checkpoint_light_client_io::{
    Error, G2TypeInfo, ReplayBackError, ReplayBackStatus, Update as SyncUpdate, G2,
};
use ethereum_beacon_client::utils;
use ethereum_common::{
    beacon::BlockHeader,
    network::Network,
    tree_hash::TreeHash,
    utils::{BootstrapResponse, FinalityUpdateResponse, Update},
};
use sails_rs::{calls::ActionIo, prelude::*};
use serde_json::Value;

const ACTOR: u64 = 1_000;
const PROGRAM: u64 = 1_001;
const LEGACY_BLS: ActorId = ActorId::new(hex_literal::hex!(
    "6b6e292c382945e80bf51af2ba7fe9f458dcff81ae6075c46f9095e1bbecdc37"
));
const SYNTHETIC: &[u8] = include_bytes!("chain-data/synthetic-fulu-transitions.json");
const HOODI: &[u8] = include_bytes!("chain-data/hoodi-boundary-492-493.json");

// The source runtime uses the legacy BLS ID; gtest 1.10 registers its new ID only.
// Both expose the same Request/Response SCALE indexes and ArkScale HOST_CALL types.
// Every request executes the real native builtin; the parent program's awaits remain queued.
#[derive(Clone, Debug)]
struct NativeBls;

impl WasmProgram for NativeBls {
    fn init(&mut self, _: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let system = System::new();
                system.mint_to(ACTOR, 100_000_000_000_000_000);
                // User-origin dry runs require a registered destination. Dispatch selects
                // the real builtin before consulting this registry entry's mock handler.
                let _native = Program::mock_with_id(&system, ::gtest::BLS12_381_ID, NativeBls);
                let reply = system
                    .calculate_reply_for_handle(
                        ACTOR,
                        ::gtest::BLS12_381_ID,
                        payload,
                        ::gtest::constants::MAX_USER_GAS_LIMIT,
                        0,
                    )
                    .expect("Native BLS builtin reply");
                assert!(reply.code.is_success(), "Native BLS builtin failed");
                Ok(Some(reply.payload))
            })
            .unwrap()
            .join()
            .unwrap()
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Err("A native builtin has no program state")
    }

    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }
}

fn large_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

fn update(data: &Value, index: usize) -> Update {
    serde::Deserialize::deserialize(&data["updates"][index]["data"]).unwrap()
}

fn full(data: &Value, index: usize) -> (SyncUpdate, Vec<u8>) {
    let update = update(data, index);
    let aggregate = update.sync_aggregate.encode();
    (
        utils::sync_update_from_update(decode_signature(&update.sync_aggregate), update),
        aggregate,
    )
}

fn finality(data: &Value, index: usize) -> (SyncUpdate, Vec<u8>) {
    let update: FinalityUpdateResponse =
        serde::Deserialize::deserialize(&data["updates"][index]).unwrap();
    let update = update.data;
    let aggregate = update.sync_aggregate.encode();
    (
        utils::sync_update_from_finality(decode_signature(&update.sync_aggregate), update),
        aggregate,
    )
}

fn headers(data: &Value) -> Vec<BlockHeader> {
    serde::Deserialize::deserialize(&data["headers"]).unwrap()
}

fn drain(system: &System, messages: &[MessageId]) -> Vec<CoreLog> {
    let mut logs = Vec::new();
    for _ in 0..100 {
        let result = system.run_next_block();
        // Event notifications also receive zero-gas automatic replies. Their failures
        // are not failures of these requested calls; require each owning reply below.
        assert!(
            messages.iter().all(|id| !result.failed.contains(id)),
            "Requested runtime dispatch failed: {:?}; logs: {:?}",
            result.failed,
            result.log
        );
        logs.extend(
            result
                .log
                .into_iter()
                .filter(|log| log.reply_to().is_some_and(|id| messages.contains(&id))),
        );
        if messages
            .iter()
            .all(|message| logs.iter().any(|log| log.reply_to() == Some(*message)))
        {
            return logs;
        }
    }
    std::panic!("Missing replies to {messages:?}");
}

fn decode_reply<T: ActionIo>(program: &Program<'_>, logs: &[CoreLog], id: MessageId) -> T::Reply {
    let reply = logs.iter().find(|log| log.reply_to() == Some(id)).unwrap();
    assert_eq!(reply.source(), program.id());
    assert_eq!(reply.destination(), ActorId::from(ACTOR));
    assert!(reply.reply_code().unwrap().is_success());
    T::decode_reply(reply.payload()).unwrap()
}

fn call<T: ActionIo>(system: &System, program: &Program<'_>, params: T::Params) -> T::Reply {
    let id = program.send_bytes(ACTOR, T::encode_call(&params));
    decode_reply::<T>(program, &drain(system, &[id]), id)
}

fn initialize<'a>(system: &'a System, data: &Value) -> Program<'a> {
    system.init_logger_with_default_filter("gwasm=debug");
    system.mint_to(ACTOR, 100_000_000_000_000_000);
    let bls = Program::mock_with_id(system, LEGACY_BLS, NativeBls);
    let id = bls.send_bytes(ACTOR, b"INIT");
    assert!(system.run_next_block().succeed.contains(&id));
    let program = Program::from_binary_with_id(system, PROGRAM, WASM_BINARY);
    let bootstrap: BootstrapResponse = serde::Deserialize::deserialize(&data["bootstrap"]).unwrap();
    let init = construct_init(Network::Hoodi, update(data, 0), bootstrap.data);
    call::<factory_io::Init>(system, &program, init);
    assert_progress(&system, &program, &update(data, 0).finalized_header);
    program
}

fn query<T: ActionIo>(system: &System, program: &Program<'_>, params: T::Params) -> T::Reply {
    let reply = system
        .calculate_reply_for_handle(
            ACTOR,
            program.id(),
            T::encode_call(&params),
            ::gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        )
        .expect("Sails query reply");
    assert!(reply.code.is_success());
    T::decode_reply(reply.payload).unwrap()
}

fn state(system: &System, program: &Program<'_>) -> StateData {
    query::<state_io::Get>(system, program, (Order::Direct, 0, 10_000))
}

fn assert_progress(system: &System, program: &Program<'_>, expected: &BlockHeader) {
    let state = state(&system, program);
    let expected_checkpoint = (
        expected.slot,
        H256::from_slice(expected.tree_hash_root().as_ref()),
    );
    assert_eq!(state.checkpoints.last(), Some(&expected_checkpoint));
    assert!(state
        .checkpoints
        .windows(2)
        .all(|pair| pair[0].0 < pair[1].0));
    assert!(state.replay_back.is_none());
    let checkpoint = query::<checkpoint_io::Get>(system, program, expected.slot).unwrap();
    assert_eq!(checkpoint, expected_checkpoint);
}

fn replay_full(
    system: &System,
    program: &Program<'_>,
    data: &Value,
    index: usize,
    ancestors: Vec<BlockHeader>,
) {
    let (update, aggregate) = full(data, index);
    assert!(matches!(
        call::<replay_io::Start>(system, program, (update, aggregate, ancestors)),
        Ok(ReplayBackStatus::Finished)
    ));
}

#[test]
fn committee_periods_advance_only_with_verified_following_keys() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        let ancestors = headers(&data);
        for use_replay in [false, true] {
            let system = System::new();
            let program = initialize(&system, &data);
            let baseline = state(&system, &program).checkpoints;
            let (update, aggregate) = finality(&data, 4);
            if use_replay {
                assert!(matches!(
                    call::<replay_io::Start>(&system, &program, (update, aggregate, Vec::new())),
                    Err(ReplayBackError::Verify(
                        Error::InvalidNextSyncCommitteeProof
                    ))
                ));
            } else {
                assert!(matches!(
                    call::<sync_io::Process>(&system, &program, (update, aggregate)),
                    Err(Error::InvalidNextSyncCommitteeProof)
                ));
            }
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());

            let (mut bad_proof, aggregate) = full(&data, 3);
            bad_proof.sync_committee_next_branch.as_mut().unwrap()[0][0] ^= 1;
            assert!(matches!(
                call::<sync_io::Process>(&system, &program, (bad_proof, aggregate)),
                Err(Error::InvalidNextSyncCommitteeProof)
            ));
            let (mut bad_signature, aggregate) = full(&data, 3);
            bad_signature.sync_committee_signature = G2TypeInfo(G2::generator()).into();
            assert!(matches!(
                call::<sync_io::Process>(&system, &program, (bad_signature, aggregate)),
                Err(Error::InvalidSignature)
            ));
            assert_eq!(state(&system, &program).checkpoints, baseline);

            if use_replay {
                replay_full(&system, &program, &data, 3, ancestors[1..3].to_vec());
            } else {
                assert!(call::<sync_io::Process>(&system, &program, full(&data, 3)).is_ok());
            }
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 4)).is_ok());
            assert_progress(&system, &program, &ancestors[4]);
            replay_full(&system, &program, &data, 5, Vec::new());
            assert_progress(&system, &program, &ancestors[5]);
            assert!(matches!(
                call::<sync_io::Process>(&system, &program, finality(&data, 7)),
                Err(Error::InvalidNextSyncCommitteeProof)
            ));
            assert!(call::<sync_io::Process>(&system, &program, full(&data, 6)).is_ok());
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 7)).is_ok());
            assert_progress(&system, &program, &ancestors[7]);
        }
    });
}

#[test]
fn duplicate_rollover_and_newer_older_async_commits_cannot_regress_history() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        let ancestors = headers(&data);
        for duplicate_rollover in [true, false] {
            let system = System::new();
            let program = initialize(&system, &data);
            let first_params = if duplicate_rollover {
                full(&data, 3)
            } else {
                finality(&data, 2)
            };
            let second_params = if duplicate_rollover {
                full(&data, 3)
            } else {
                finality(&data, 1)
            };
            // Enqueue both callers before any of their four BLS reply awaits can complete.
            let first = program.send_bytes(
                ACTOR,
                <sync_io::Process as ActionIo>::encode_call(&first_params),
            );
            let second = program.send_bytes(
                ACTOR,
                <sync_io::Process as ActionIo>::encode_call(&second_params),
            );
            let logs = drain(&system, &[first, second]);
            assert!(decode_reply::<sync_io::Process>(&program, &logs, first).is_ok());
            assert!(matches!(
                decode_reply::<sync_io::Process>(&program, &logs, second),
                Err(Error::StateChanged)
            ));
            let index = if duplicate_rollover { 3 } else { 2 };
            assert_progress(&system, &program, &ancestors[index]);
            assert_eq!(state(&system, &program).checkpoints.len(), 2);
            if !duplicate_rollover {
                assert!(call::<sync_io::Process>(&system, &program, full(&data, 3)).is_ok());
            }
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 4)).is_ok());
            replay_full(&system, &program, &data, 5, Vec::new());
            assert!(call::<sync_io::Process>(&system, &program, full(&data, 6)).is_ok());
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 7)).is_ok());
            assert_progress(&system, &program, &ancestors[7]);
        }
    });
}

#[test]
fn overlapping_replays_preserve_first_progress_and_immutable_normal_base() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        let ancestors = headers(&data);
        // Replay first, normal first, two incomplete starts, and completion before a stale start.
        for schedule in 0..4 {
            let system = System::new();
            let program = initialize(&system, &data);
            let (update, aggregate) = full(&data, 3);
            let replay_headers = if schedule == 3 {
                ancestors[1..3].to_vec()
            } else {
                vec![ancestors[2].clone()]
            };
            let replay_payload =
                <replay_io::Start as ActionIo>::encode_call(&(update, aggregate, replay_headers));
            let normal_payload = <sync_io::Process as ActionIo>::encode_call(&finality(&data, 2));
            let (second_update, second_aggregate) = full(&data, 4);
            let later_replay = <replay_io::Start as ActionIo>::encode_call(&(
                second_update,
                second_aggregate,
                Vec::new(),
            ));
            let (first_payload, second_payload) = match schedule {
                0 => (replay_payload, normal_payload),
                1 => (normal_payload, replay_payload),
                _ => (replay_payload, later_replay),
            };
            let first = program.send_bytes(ACTOR, first_payload);
            let second = program.send_bytes(ACTOR, second_payload);
            let logs = drain(&system, &[first, second]);
            match schedule {
                0 => {
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, first),
                        Ok(ReplayBackStatus::InProcess)
                    ));
                    assert!(matches!(
                        decode_reply::<sync_io::Process>(&program, &logs, second),
                        Err(Error::StateChanged)
                    ));
                }
                1 => {
                    assert!(decode_reply::<sync_io::Process>(&program, &logs, first).is_ok());
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, second),
                        Err(ReplayBackError::Verify(Error::StateChanged))
                    ));
                    assert_progress(&system, &program, &ancestors[2]);
                    replay_full(&system, &program, &data, 3, Vec::new());
                }
                2 => {
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, first),
                        Ok(ReplayBackStatus::InProcess)
                    ));
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, second),
                        Err(ReplayBackError::AlreadyStarted)
                    ));
                }
                3 => {
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, first),
                        Ok(ReplayBackStatus::Finished)
                    ));
                    assert!(matches!(
                        decode_reply::<replay_io::Start>(&program, &logs, second),
                        Err(ReplayBackError::Verify(Error::StateChanged))
                    ));
                    assert_progress(&system, &program, &ancestors[3]);
                }
                _ => unreachable!(),
            }
            if matches!(schedule, 0 | 2) {
                let progress = state(&system, &program);
                let replay = progress.replay_back.unwrap();
                assert_eq!(replay.finalized_header, ancestors[3].slot);
                assert_eq!(replay.last_header, ancestors[2].slot);
                assert_eq!(
                    progress.checkpoints,
                    vec![(
                        ancestors[0].slot,
                        H256::from_slice(ancestors[0].tree_hash_root().as_ref())
                    )]
                );
                assert!(matches!(
                    call::<sync_io::Process>(&system, &program, finality(&data, 2)),
                    Err(Error::ReplayBackRequired {
                        replay_back: Some(_),
                        ..
                    })
                ));
                assert_eq!(state(&system, &program).replay_back.unwrap(), replay);
                assert!(matches!(
                    call::<replay_io::Process>(&system, &program, vec![ancestors[1].clone()]),
                    Ok(ReplayBackStatus::Finished)
                ));
            }
            assert_progress(&system, &program, &ancestors[3]);
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 4)).is_ok());
            assert_progress(&system, &program, &ancestors[4]);
        }
    });
}

#[test]
fn genuine_hoodi_boundary_rejects_finality_without_following_committee() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(HOODI).unwrap();
        let ancestors = headers(&data);
        let system = System::new();
        let program = initialize(&system, &data);
        for index in [1, 2] {
            let baseline = state(&system, &program).checkpoints;
            let (finality_update, aggregate) = finality(&data, index);
            assert!(matches!(
                call::<replay_io::Start>(
                    &system,
                    &program,
                    (finality_update, aggregate, Vec::new())
                ),
                Err(ReplayBackError::Verify(
                    Error::InvalidNextSyncCommitteeProof
                ))
            ));
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
            let previous = update(&data, index - 1).finalized_header;
            let finalized = update(&data, index).finalized_header;
            let replay_headers = ancestors
                .iter()
                .rev()
                .filter(|header| previous.slot < header.slot && header.slot < finalized.slot)
                .cloned()
                .collect::<Vec<_>>();
            let mut batches = replay_headers.chunks(96);
            let (full_update, aggregate) = full(&data, index);
            let mut result = call::<replay_io::Start>(
                &system,
                &program,
                (
                    full_update,
                    aggregate,
                    batches.next().unwrap().iter().rev().cloned().collect(),
                ),
            )
            .unwrap();
            for batch in batches {
                assert!(matches!(result, ReplayBackStatus::InProcess));
                result = call::<replay_io::Process>(
                    &system,
                    &program,
                    batch.iter().rev().cloned().collect(),
                )
                .unwrap();
            }
            assert!(matches!(result, ReplayBackStatus::Finished));
            assert!(state(&system, &program).replay_back.is_none());
            assert_progress(&system, &program, &finalized);
            // The authentic signature now reaches actuality through the rotated committee.
            assert!(matches!(
                call::<sync_io::Process>(&system, &program, finality(&data, index)),
                Err(Error::NotActual)
            ));
        }
    });
}

#[test]
fn off_curve_following_committee_cannot_poison_normal_or_replay_progress() {
    use ark_ec::CurveGroup;
    use ark_serialize::CanonicalSerialize;

    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        let ancestors = headers(&data);
        for replay in [false, true] {
            let system = System::new();
            let program = initialize(&system, &data);
            let baseline = state(&system, &program).checkpoints;
            let (mut poisoned, aggregate) = full(&data, 3);
            let point = &mut poisoned.sync_committee_next_pub_keys.as_mut().unwrap().0[0]
                .0
                 .0;
            let mut authentic = Vec::new();
            point.serialize_compressed(&mut authentic).unwrap();
            point.y = ark_bls12_381::Fq::from(1u64);
            assert!(!point.into_affine().is_on_curve());
            let mut substituted = Vec::new();
            point.serialize_compressed(&mut substituted).unwrap();
            assert_eq!(substituted, authentic);
            if replay {
                assert!(matches!(
                    call::<replay_io::Start>(
                        &system,
                        &program,
                        (poisoned, aggregate, ancestors[1..3].to_vec())
                    ),
                    Err(ReplayBackError::Verify(Error::InvalidPublicKeys))
                ));
            } else {
                assert!(matches!(
                    call::<sync_io::Process>(&system, &program, (poisoned, aggregate)),
                    Err(Error::InvalidPublicKeys)
                ));
            }
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
            assert!(call::<sync_io::Process>(&system, &program, full(&data, 3)).is_ok());
            assert!(call::<sync_io::Process>(&system, &program, finality(&data, 4)).is_ok());
            assert_progress(&system, &program, &ancestors[4]);
        }
    });
}

#[test]
fn first_fulu_signature_slot_uses_the_preceding_domain_for_both_callers() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(include_bytes!(
            "chain-data/synthetic-hoodi-fulu-signature-boundary.json"
        ))
        .unwrap();
        let ancestors = headers(&data);
        let mut wrong_domain = data.clone();
        wrong_domain["updates"][1]["data"]["sync_aggregate"]["sync_committee_signature"] =
            data["wrong_domain_signature"].clone();
        for replay in [false, true] {
            let system = System::new();
            let program = initialize(&system, &data);
            let baseline = state(&system, &program).checkpoints;
            let (invalid, aggregate) = full(&wrong_domain, 1);
            if replay {
                assert!(matches!(
                    call::<replay_io::Start>(
                        &system,
                        &program,
                        (invalid, aggregate, ancestors[1..2].to_vec())
                    ),
                    Err(ReplayBackError::Verify(Error::InvalidSignature))
                ));
            } else {
                assert!(matches!(
                    call::<sync_io::Process>(&system, &program, (invalid, aggregate)),
                    Err(Error::InvalidSignature)
                ));
            }
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
            if replay {
                replay_full(&system, &program, &data, 1, ancestors[1..2].to_vec());
            } else {
                assert!(call::<sync_io::Process>(&system, &program, full(&data, 1)).is_ok());
            }
            assert_progress(&system, &program, &ancestors[2]);
        }
    });
}

#[test]
fn off_curve_signature_returns_an_error_without_losing_checkpoint_state() {
    use ark_ec::CurveGroup;

    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        for replay in [false, true] {
            let system = System::new();
            let program = initialize(&system, &data);
            let baseline = state(&system, &program).checkpoints;
            let (mut invalid, aggregate) = full(&data, 3);
            let signature = &mut invalid.sync_committee_signature.0 .0;
            signature.x = Default::default();
            signature.y = Default::default();
            assert!(!signature.into_affine().is_on_curve());
            if replay {
                assert!(matches!(
                    call::<replay_io::Start>(
                        &system,
                        &program,
                        (invalid, aggregate, headers(&data)[1..3].to_vec())
                    ),
                    Err(ReplayBackError::Verify(Error::InvalidSignature))
                ));
            } else {
                assert!(matches!(
                    call::<sync_io::Process>(&system, &program, (invalid, aggregate)),
                    Err(Error::InvalidSignature)
                ));
            }
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
            assert!(call::<sync_io::Process>(&system, &program, full(&data, 3)).is_ok());
        }
    });
}

#[test]
fn trusted_bootstrap_identity_period_and_canonical_encoding_fail_closed() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        for case in 0..5 {
            let system = System::new();
            system.mint_to(ACTOR, 100_000_000_000_000_000);
            let program = Program::from_binary_with_id(&system, PROGRAM, WASM_BINARY);
            let bootstrap: BootstrapResponse =
                serde::Deserialize::deserialize(&data["bootstrap"]).unwrap();
            let mut init = construct_init(Network::Hoodi, update(&data, 0), bootstrap.data);
            match case {
                0 => init.trusted_bootstrap_root.0[0] ^= 1,
                1 => {
                    init.bootstrap_header.slot = 0;
                    init.update.finalized_header.slot = 0;
                    init.trusted_bootstrap_root = init.bootstrap_header.tree_hash_root();
                }
                2 => init.network = Network::Mainnet,
                3 => init.sync_aggregate_encoded.push(0),
                _ => (),
            }
            let mut payload = <factory_io::Init as ActionIo>::encode_call(&init);
            if case == 4 {
                payload.push(0);
            }
            let id = program.send_bytes(ACTOR, payload);
            let result = system.run_next_block();
            assert!(
                result.failed.contains(&id),
                "invalid bootstrap case {case} activated"
            );
        }
    });
}

#[test]
fn future_signatures_skipped_periods_and_trailing_scale_reject_both_updates() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        for replay in [false, true] {
            let system = System::new();
            let program = initialize(&system, &data);
            let baseline = state(&system, &program).checkpoints;
            for case in 0..3 {
                let (mut update, mut aggregate) = full(&data, 3);
                let expected = match case {
                    0 => {
                        update.signature_slot = Network::Hoodi
                            .current_slot(system.block_timestamp())
                            .unwrap()
                            + 10_000;
                        Error::InvalidTimestamp
                    }
                    1 => {
                        update.signature_slot = (ethereum_common::utils::calculate_period(
                            update.finalized_header.slot,
                        ) + 1)
                            * 8_192
                            + 2;
                        update.attested_header.slot = update.signature_slot - 1;
                        update.finalized_header.slot = update.signature_slot - 2;
                        Error::InvalidPeriod
                    }
                    _ => {
                        aggregate.push(0);
                        Error::InvalidSyncAggregate
                    }
                };
                if replay {
                    let result = call::<replay_io::Start>(
                        &system,
                        &program,
                        (update, aggregate, Vec::new()),
                    );
                    assert!(
                        matches!(result, Err(ReplayBackError::Verify(error)) if error.encode() == expected.encode())
                    );
                } else {
                    let result = call::<sync_io::Process>(&system, &program, (update, aggregate));
                    // Large gaps request authenticated replay; no committee interval is skipped.
                    assert!(
                        matches!(result, Err(Error::ReplayBackRequired { .. })) && case == 1
                            || matches!(result, Err(error) if error.encode() == expected.encode())
                    );
                }
                assert_eq!(state(&system, &program).checkpoints, baseline);
                assert!(state(&system, &program).replay_back.is_none());
            }
            let (update, aggregate) = full(&data, 3);
            let mut payload = if replay {
                <replay_io::Start as ActionIo>::encode_call(&(
                    update,
                    aggregate,
                    Vec::<BlockHeader>::new(),
                ))
            } else {
                <sync_io::Process as ActionIo>::encode_call(&(update, aggregate))
            };
            payload.push(0);
            let id = program.send_bytes(ACTOR, payload);
            // Pinned Sails InvocationIo::decode_params uses DecodeAll before dispatch.
            assert!(system.run_next_block().failed.contains(&id));
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
            assert!(call::<sync_io::Process>(&system, &program, full(&data, 3)).is_ok());
        }
    });
}

#[test]
fn replay_batches_reject_order_duplicates_and_unbound_endpoints_atomically() {
    large_stack(|| {
        let data: Value = serde_json::from_slice(SYNTHETIC).unwrap();
        let ancestors = headers(&data);
        let system = System::new();
        let program = initialize(&system, &data);
        let baseline = state(&system, &program).checkpoints;
        for case in 0..4 {
            let mut batch = ancestors[1..3].to_vec();
            match case {
                0 => batch.reverse(),
                1 => batch.push(batch[1].clone()),
                2 => batch[0].state_root.0[0] ^= 1,
                _ => batch.push(ancestors[3].clone()),
            }
            let (update, aggregate) = full(&data, 3);
            assert!(matches!(
                call::<replay_io::Start>(&system, &program, (update, aggregate, batch)),
                Err(ReplayBackError::Verify(Error::InvalidHeaders))
            ));
            assert_eq!(state(&system, &program).checkpoints, baseline);
            assert!(state(&system, &program).replay_back.is_none());
        }
        let (update, aggregate) = full(&data, 3);
        assert!(matches!(
            call::<replay_io::Start>(
                &system,
                &program,
                (update, aggregate, vec![ancestors[2].clone()])
            ),
            Ok(ReplayBackStatus::InProcess)
        ));
        let progress = state(&system, &program).replay_back.unwrap();
        let mut invalid = ancestors[1].clone();
        invalid.parent_root.0[0] ^= 1;
        assert!(matches!(
            call::<replay_io::Process>(&system, &program, vec![invalid]),
            Err(ReplayBackError::Verify(Error::InvalidHeaders))
        ));
        assert_eq!(state(&system, &program).replay_back.unwrap(), progress);
        assert!(matches!(
            call::<replay_io::Process>(&system, &program, ancestors[..2].to_vec()),
            Ok(ReplayBackStatus::Finished)
        ));
        assert_progress(&system, &program, &ancestors[3]);
        assert!(matches!(
            query::<state_io::Network>(&system, &program, ()),
            Network::Hoodi
        ));
    });
}

#[test]
fn network_clock_uses_milliseconds_and_rejects_pre_genesis_time() {
    for network in [
        Network::Mainnet,
        Network::Sepolia,
        Network::Holesky,
        Network::Hoodi,
    ] {
        let genesis = network.genesis_time() * 1_000;
        assert_eq!(network.current_slot(genesis - 1), None);
        assert_eq!(network.current_slot(genesis), Some(0));
        assert_eq!(network.current_slot(genesis + 11_999), Some(0));
        assert_eq!(network.current_slot(genesis + 12_000), Some(1));
        assert_eq!(network.current_slot(genesis + 24_001), Some(2));
    }
}
