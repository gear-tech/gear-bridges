use alloy_consensus::Receipt;
use alloy_primitives::{Address, Bytes, Log as EthLog};
use alloy_rlp::Encodable;
use alloy_sol_types::{sol, SolEvent};
use ethereum_common::utils::ReceiptEnvelope;
use gtest::{CoreLog, Log, Program, System, WasmProgram};
use parity_scale_codec::DecodeAll;
use ping::{BridgeConfig, Delivery, OutboundStatus, PingError, QueuedDelivery};
use sails_rs::prelude::{ActorId, Decode, Encode, MessageId, H160, H256, U256};

include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

const OWNER: u64 = 1000;
const PROXY: u64 = 500;
const BUILTIN: u64 = 300;
const PROGRAM: u64 = 600;
const EMITTER: H160 = H160([1; 20]);
const SENDER: H160 = H160([2; 20]);
const FEE: u128 = 1_000_000_000_000;

sol! {
    event MessageRequested(bytes32 indexed applicationId, address indexed sender, bytes32 indexed destination, bytes payload);
}

fn config(fee_bridge: u128) -> BridgeConfig {
    BridgeConfig {
        builtin: BUILTIN.into(),
        fee_bridge,
        gas_to_send_request_to_builtin: 10_000_000_000,
        gas_for_reply_deposit: 10_000_000_000,
        reply_timeout: 3,
    }
}

fn original_reply(system: &System, id: MessageId) -> CoreLog {
    for _ in 0..120 {
        if let Some(log) = system
            .run_next_block()
            .log
            .into_iter()
            .find(|log| log.reply_to() == Some(id))
        {
            return log;
        }
    }
    panic!("Missing original reply to {id:?}");
}

fn initialize(system: &System, bridge_config: BridgeConfig) -> Program<'_> {
    for actor in [OWNER, PROXY] {
        system.mint_to(actor, 100_000_000_000_000_000);
    }
    if system.get_program(BUILTIN).is_none() {
        system.mint_to(BUILTIN, 100_000_000_000_000_000);
    }
    let program = Program::from_binary_with_id(system, PROGRAM, WASM_BINARY_OPT);
    let init = program.send_bytes(
        OWNER,
        (
            "New",
            ActorId::from(PROXY),
            EMITTER,
            SENDER,
            EMITTER,
            bridge_config,
        )
            .encode(),
    );
    assert!(original_reply(system, init)
        .reply_code()
        .unwrap()
        .is_success());
    program
}

fn decode_reply<T: Decode>(payload: &[u8], method: &str) -> T {
    let (service, route, result) = <(String, String, T)>::decode_all(&mut &payload[..]).unwrap();
    assert_eq!(service, "Ping");
    assert_eq!(route, method);
    result
}

fn call<T: Decode>(
    system: &System,
    program: &Program<'_>,
    source: u64,
    method: &str,
    args: impl Encode,
    value: u128,
) -> T {
    let id = program.send_bytes_with_value(source, ("Ping", method, args).encode(), value);
    let reply = original_reply(system, id);
    assert_eq!(reply.source(), program.id());
    assert_eq!(reply.destination(), ActorId::from(source));
    assert!(
        reply.reply_code().unwrap().is_success(),
        "Unexpected runtime error: {reply:?}"
    );
    decode_reply(reply.payload(), method)
}

fn query<T: Decode>(system: &System, program: &Program<'_>, method: &str, args: impl Encode) -> T {
    let reply = system
        .calculate_reply_for_handle(
            OWNER,
            program.id(),
            ("Ping", method, args).encode(),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        )
        .unwrap();
    assert!(reply.code.is_success());
    decode_reply(&reply.payload, method)
}

fn event(emitter: H160, sender: H160, destination: ActorId, id: H256, payload: &[u8]) -> EthLog {
    let data = MessageRequested {
        applicationId: id.0.into(),
        sender: sender.0.into(),
        destination: destination.into_bytes().into(),
        payload: Bytes::copy_from_slice(payload),
    }
    .encode_log_data();
    EthLog {
        address: Address::from(emitter.0),
        data,
    }
}

fn receipt(success: bool, logs: Vec<EthLog>) -> Vec<u8> {
    let receipt = ReceiptEnvelope::Eip1559(
        Receipt {
            status: success.into(),
            cumulative_gas_used: 42_000,
            logs,
        }
        .with_bloom(),
    );
    let mut bytes = Vec::new();
    receipt.encode(&mut bytes);
    bytes
}

#[test]
fn receipt_authentication_exact_payload_and_two_replay_keys() {
    let system = System::new();
    let program = initialize(&system, config(0));
    let id = H256([7; 32]);
    let key = (17_u64, u64::from(u32::MAX) + 5);
    let payload = [0, 1, 255, 112, 105, 110, 103, 0];
    let valid_log = event(EMITTER, SENDER, program.id(), id, &payload);
    let valid = receipt(true, vec![valid_log.clone()]);
    assert_eq!(
        call::<Result<Delivery, PingError>>(
            &system,
            &program,
            OWNER,
            "SubmitReceipt",
            (key.0, key.1, vec![0xff_u8]),
            0
        ),
        Err(PingError::NotHistoricalProxy)
    );

    let mut trailing_receipt = valid.clone();
    trailing_receipt.push(0);
    let mut malformed_log = valid_log.clone();
    malformed_log.data.data = Bytes::from(vec![0xff]);
    let mut noncanonical_sender = valid_log.clone();
    noncanonical_sender.data.topics_mut()[2][0] = 1;
    let mut trailing_log = valid_log.clone();
    let mut data = trailing_log.data.data.to_vec();
    data.extend([0_u8; 32]);
    trailing_log.data.data = Bytes::from(data);
    let mut padded_log = valid_log.clone();
    let mut data = padded_log.data.data.to_vec();
    *data.last_mut().unwrap() = 1;
    padded_log.data.data = Bytes::from(data);
    let mut root_only_receipt = Vec::new();
    ReceiptEnvelope::Eip1559(
        Receipt {
            status: alloy_primitives::B256::ZERO.into(),
            cumulative_gas_used: 42_000,
            logs: vec![valid_log.clone()],
        }
        .with_bloom(),
    )
    .encode(&mut root_only_receipt);
    let rejected = [
        (
            receipt(
                true,
                vec![event(H160([9; 20]), SENDER, program.id(), id, &payload)],
            ),
            PingError::UnsupportedEvent,
        ),
        (
            receipt(
                true,
                vec![event(EMITTER, H160([9; 20]), program.id(), id, &payload)],
            ),
            PingError::WrongSender,
        ),
        (
            receipt(
                true,
                vec![event(EMITTER, SENDER, OWNER.into(), id, &payload)],
            ),
            PingError::WrongDestination,
        ),
        (
            receipt(false, vec![valid_log.clone()]),
            PingError::InvalidReceipt,
        ),
        (root_only_receipt, PingError::InvalidReceipt),
        (trailing_receipt, PingError::InvalidReceipt),
        (valid[..valid.len() - 1].to_vec(), PingError::InvalidReceipt),
        (
            receipt(true, vec![malformed_log]),
            PingError::InvalidReceipt,
        ),
        (
            receipt(true, vec![noncanonical_sender]),
            PingError::InvalidReceipt,
        ),
        (receipt(true, vec![trailing_log]), PingError::InvalidReceipt),
        (receipt(true, vec![padded_log]), PingError::InvalidReceipt),
        (
            receipt(true, vec![valid_log.clone(), valid_log]),
            PingError::AmbiguousDelivery,
        ),
        (
            receipt(
                true,
                vec![event(EMITTER, SENDER, program.id(), H256::zero(), &payload)],
            ),
            PingError::InvalidPayload,
        ),
        (
            receipt(
                true,
                vec![event(EMITTER, SENDER, program.id(), id, &vec![0; 1025])],
            ),
            PingError::InvalidPayload,
        ),
    ];
    for (bytes, error) in rejected {
        assert_eq!(
            call::<Result<Delivery, PingError>>(
                &system,
                &program,
                PROXY,
                "SubmitReceipt",
                (key.0, key.1, bytes),
                0
            ),
            Err(error)
        );
        assert_eq!(
            query::<Option<Delivery>>(&system, &program, "Received", key),
            None
        );
        assert_eq!(
            query::<Option<Vec<u8>>>(&system, &program, "PayloadOf", id),
            None
        );
    }
    let delivery = Delivery {
        application_id: id,
        payload: payload.to_vec(),
    };
    assert_eq!(
        call::<Result<Delivery, PingError>>(
            &system,
            &program,
            PROXY,
            "SubmitReceipt",
            (key.0, key.1, valid.clone()),
            0
        ),
        Ok(delivery.clone())
    );
    assert_eq!(
        query::<Option<Delivery>>(&system, &program, "Received", key),
        Some(delivery)
    );
    assert_eq!(
        query::<Option<Vec<u8>>>(&system, &program, "PayloadOf", id),
        Some(payload.to_vec())
    );
    assert_eq!(
        call::<Result<Delivery, PingError>>(
            &system,
            &program,
            PROXY,
            "SubmitReceipt",
            (key.0, key.1, valid.clone()),
            0
        ),
        Err(PingError::AlreadyProcessed)
    );
    assert_eq!(
        call::<Result<Delivery, PingError>>(
            &system,
            &program,
            PROXY,
            "SubmitReceipt",
            (key.0, key.1 + 1, valid),
            0
        ),
        Err(PingError::AlreadyReceived)
    );
    assert_eq!(
        query::<Option<Delivery>>(&system, &program, "Received", (key.0, key.1 + 1)),
        None
    );
    assert_eq!(
        query::<Option<Vec<u8>>>(&system, &program, "PayloadOf", id),
        Some(payload.to_vec())
    );

    for (offset, payload) in [Vec::new(), vec![0xff; 1024]].into_iter().enumerate() {
        let id = H256::from_low_u64_be(offset as u64 + 1);
        let bytes = receipt(
            true,
            vec![event(EMITTER, SENDER, program.id(), id, &payload)],
        );
        assert_eq!(
            call::<Result<Delivery, PingError>>(
                &system,
                &program,
                PROXY,
                "SubmitReceipt",
                (18_u64, offset as u64, bytes),
                0
            ),
            Ok(Delivery {
                application_id: id,
                payload: payload.clone()
            })
        );
        assert_eq!(
            query::<Option<Vec<u8>>>(&system, &program, "PayloadOf", id),
            Some(payload)
        );
    }
}

fn queued() -> QueuedDelivery {
    QueuedDelivery {
        block_number: 12,
        hash: H256([9; 32]),
        nonce: U256::zero(),
        queue_id: 17,
    }
}

fn queued_bytes() -> Vec<u8> {
    let delivery = queued();
    gbuiltin_eth_bridge::Response::EthMessageQueued {
        block_number: delivery.block_number,
        hash: delivery.hash,
        nonce: delivery.nonce,
        queue_id: delivery.queue_id,
    }
    .encode()
}

#[derive(Clone, Debug)]
struct BuiltinReply {
    expected_payload: Vec<u8>,
    response: Result<Vec<u8>, &'static str>,
}

impl WasmProgram for BuiltinReply {
    fn init(&mut self, _: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }
    fn handle(&mut self, payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        let request = gbuiltin_eth_bridge::Request::decode_all(&mut &payload[..]).unwrap();
        assert_eq!(
            request,
            gbuiltin_eth_bridge::Request::SendEthMessage {
                destination: EMITTER,
                payload: self.expected_payload.clone()
            }
        );
        self.response.clone().map(Some)
    }
    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }
    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }
}

#[test]
fn builtin_original_reply_nonce_zero_value_and_dispatch_replay() {
    let system = System::new();
    let id = H256([3; 32]);
    system.mint_to(OWNER, 100_000_000_000_000_000);
    let payload = vec![255, 0, 2, 112, 105, 110, 103, 0];
    let expected_payload = [id.as_bytes(), &payload].concat();
    let builtin = Program::mock_with_id(
        &system,
        BUILTIN,
        BuiltinReply {
            expected_payload,
            response: Ok(queued_bytes()),
        },
    );
    builtin.send_bytes(OWNER, b"INIT");
    let program = initialize(&system, config(FEE));
    let before = builtin.balance();
    assert_eq!(
        call::<Result<QueuedDelivery, PingError>>(
            &system,
            &program,
            OWNER,
            "SendMessage",
            (id, payload.clone()),
            FEE
        ),
        Ok(queued())
    );
    assert_eq!(builtin.balance(), before + FEE);
    assert_eq!(
        query::<Option<OutboundStatus>>(&system, &program, "Outbound", id),
        Some(OutboundStatus::Queued(queued()))
    );
    assert_eq!(
        call::<Result<QueuedDelivery, PingError>>(
            &system,
            &program,
            OWNER,
            "SendMessage",
            (id, payload),
            FEE
        ),
        Err(PingError::AlreadyRequested)
    );
    assert_eq!(builtin.balance(), before + FEE);
    assert_eq!(
        call::<Result<QueuedDelivery, PingError>>(
            &system,
            &program,
            PROXY,
            "SendMessage",
            (H256([4; 32]), vec![1_u8]),
            0
        ),
        Err(PingError::NotOwner)
    );
    for (id, payload) in [(H256::zero(), vec![1_u8]), (H256([4; 32]), vec![0; 1025])] {
        assert_eq!(
            call::<Result<QueuedDelivery, PingError>>(
                &system,
                &program,
                OWNER,
                "SendMessage",
                (id, payload),
                FEE
            ),
            Err(PingError::InvalidPayload)
        );
        assert_eq!(
            query::<Option<OutboundStatus>>(&system, &program, "Outbound", id),
            None
        );
    }
    let mismatch_id = H256([5; 32]);
    let before = program.balance();
    let request = program.send_bytes_with_value(
        OWNER,
        ("Ping", "SendMessage", mismatch_id, vec![1_u8]).encode(),
        FEE * 2,
    );
    assert!(!original_reply(&system, request)
        .reply_code()
        .unwrap()
        .is_success());
    assert_eq!(program.balance(), before);
    assert_eq!(
        query::<Option<OutboundStatus>>(&system, &program, "Outbound", mismatch_id),
        None
    );
}

#[test]
fn malformed_or_failed_builtin_reply_never_reopens_original_dispatch() {
    let mut trailing = queued_bytes();
    trailing.push(0);
    for (response, error) in [
        (Ok(trailing), PingError::BuiltinDecode),
        (Err("rejected"), PingError::BuiltinFailure),
    ] {
        let system = System::new();
        system.mint_to(OWNER, 100_000_000_000_000_000);
        let id = H256([3; 32]);
        let builtin = Program::mock_with_id(
            &system,
            BUILTIN,
            BuiltinReply {
                expected_payload: [id.as_bytes(), &[1_u8]].concat(),
                response,
            },
        );
        builtin.send_bytes(OWNER, b"INIT");
        let program = initialize(&system, config(0));
        assert_eq!(
            call::<Result<QueuedDelivery, PingError>>(
                &system,
                &program,
                OWNER,
                "SendMessage",
                (id, vec![1_u8]),
                0
            ),
            Err(error)
        );
        assert_eq!(
            query::<Option<OutboundStatus>>(&system, &program, "Outbound", id),
            Some(OutboundStatus::Pending)
        );
        assert_eq!(
            call::<Result<QueuedDelivery, PingError>>(
                &system,
                &program,
                OWNER,
                "SendMessage",
                (id, vec![1_u8]),
                0
            ),
            Err(PingError::AlreadyRequested)
        );
    }
}

#[test]
fn late_original_builtin_reply_reconciles_pending_after_timeout() {
    let system = System::new();
    let program = initialize(&system, config(0));
    let id = H256([3; 32]);
    assert_eq!(
        call::<Result<QueuedDelivery, PingError>>(
            &system,
            &program,
            OWNER,
            "SendMessage",
            (id, vec![1_u8]),
            0
        ),
        Err(PingError::BuiltinFailure)
    );
    assert_eq!(
        query::<Option<OutboundStatus>>(&system, &program, "Outbound", id),
        Some(OutboundStatus::Pending)
    );
    let original = Log::builder()
        .source(program.id())
        .dest(BUILTIN)
        .payload_bytes(
            gbuiltin_eth_bridge::Request::SendEthMessage {
                destination: EMITTER,
                payload: [id.as_bytes(), &[1_u8]].concat(),
            }
            .encode(),
        );
    let mailbox = system.get_mailbox(BUILTIN);
    assert!(mailbox.contains(&original));
    mailbox.reply_bytes(original, queued_bytes(), 0).unwrap();
    for _ in 0..3 {
        system.run_next_block();
    }
    assert_eq!(
        query::<Option<OutboundStatus>>(&system, &program, "Outbound", id),
        Some(OutboundStatus::Queued(queued()))
    );
    assert_eq!(
        call::<Result<QueuedDelivery, PingError>>(
            &system,
            &program,
            OWNER,
            "SendMessage",
            (id, vec![1_u8]),
            0
        ),
        Err(PingError::AlreadyRequested)
    );
}

#[test]
fn initialization_rejects_unbound_identities_and_invalid_immutable_gas() {
    let system = System::new();
    system.mint_to(OWNER, 100_000_000_000_000_000);
    let valid = config(0);
    let cases = [
        (ActorId::zero(), EMITTER, SENDER, EMITTER, valid),
        (PROXY.into(), H160::zero(), SENDER, EMITTER, valid),
        (PROXY.into(), EMITTER, H160::zero(), EMITTER, valid),
        (PROXY.into(), EMITTER, SENDER, H160::zero(), valid),
        (PROXY.into(), EMITTER, SENDER, H160([9; 20]), valid),
        (
            PROXY.into(),
            EMITTER,
            SENDER,
            EMITTER,
            BridgeConfig {
                builtin: ActorId::zero(),
                ..valid
            },
        ),
        (
            PROXY.into(),
            EMITTER,
            SENDER,
            EMITTER,
            BridgeConfig {
                gas_to_send_request_to_builtin: 0,
                ..valid
            },
        ),
        (
            PROXY.into(),
            EMITTER,
            SENDER,
            EMITTER,
            BridgeConfig {
                gas_for_reply_deposit: 0,
                ..valid
            },
        ),
        (
            PROXY.into(),
            EMITTER,
            SENDER,
            EMITTER,
            BridgeConfig {
                reply_timeout: 0,
                ..valid
            },
        ),
    ];
    for (offset, (proxy, emitter, sender, receiver, bridge_config)) in cases.into_iter().enumerate()
    {
        let program =
            Program::from_binary_with_id(&system, PROGRAM + offset as u64, WASM_BINARY_OPT);
        let id = program.send_bytes(
            OWNER,
            ("New", proxy, emitter, sender, receiver, bridge_config).encode(),
        );
        assert!(!original_reply(&system, id)
            .reply_code()
            .unwrap()
            .is_success());
        assert!(!system.is_active_program(program.id()));
    }
}
