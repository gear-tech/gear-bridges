#![no_std]

extern crate alloc;

use alloc::{collections::BTreeMap, rc::Rc};
use alloy_rlp::Decodable;
use alloy_sol_types::SolEvent;
use core::cell::RefCell;
use ethereum_common::utils::ReceiptEnvelope;
use gstd::{msg, MessageId};
use sails_rs::prelude::*;

mod abi {
    alloy_sol_types::sol! {
        event MessageRequested(
            bytes32 indexed applicationId,
            address indexed sender,
            bytes32 indexed destination,
            bytes payload
        );
    }
}

const MAX_PAYLOAD: usize = 1024;

#[derive(Clone, Copy, Debug, Encode, Decode, TypeInfo)]
pub struct BridgeConfig {
    pub builtin: ActorId,
    pub fee_bridge: u128,
    pub gas_to_send_request_to_builtin: u64,
    pub gas_for_reply_deposit: u64,
    pub reply_timeout: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct Delivery {
    pub application_id: H256,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct QueuedDelivery {
    pub block_number: u32,
    pub hash: H256,
    pub nonce: U256,
    pub queue_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum OutboundStatus {
    Pending,
    Queued(QueuedDelivery),
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum PingError {
    NotHistoricalProxy,
    NotOwner,
    InvalidReceipt,
    UnsupportedEvent,
    WrongSender,
    WrongDestination,
    InvalidPayload,
    AmbiguousDelivery,
    AlreadyProcessed,
    AlreadyReceived,
    AlreadyRequested,
    BuiltinFailure,
    BuiltinDecode,
}

#[event]
#[derive(Encode, Decode, TypeInfo)]
pub enum Event {
    ReceiptSubmitted {
        slot: u64,
        transaction_index: u64,
        application_id: H256,
        payload: Vec<u8>,
    },
}

struct OutboundRequest {
    request_message_id: MessageId,
    builtin_message_id: Option<MessageId>,
    reply_message_id: Option<MessageId>,
    result: Option<Result<QueuedDelivery, PingError>>,
}

struct State {
    owner: ActorId,
    historical_proxy: ActorId,
    ethereum_emitter: H160,
    ethereum_sender: H160,
    ethereum_receiver: H160,
    bridge_config: BridgeConfig,
    receipts: BTreeMap<(u64, u64), Delivery>,
    received_ids: BTreeMap<H256, (u64, u64)>,
    outbound: BTreeMap<H256, OutboundRequest>,
}

pub struct PingService {
    state: Rc<RefCell<State>>,
}

#[service(events = Event)]
impl PingService {
    #[export]
    pub fn submit_receipt(
        &mut self,
        slot: u64,
        transaction_index: u64,
        receipt_rlp: Vec<u8>,
    ) -> Result<Delivery, PingError> {
        if msg::source() != self.state.borrow().historical_proxy {
            return Err(PingError::NotHistoricalProxy);
        }
        assert_eq!(msg::value(), 0, "Receipt submission must not attach value");
        let delivery = {
            let state = self.state.borrow();
            decode_delivery(&state, &receipt_rlp)?
        };
        let key = (slot, transaction_index);
        {
            let mut state = self.state.borrow_mut();
            if state.receipts.contains_key(&key) {
                return Err(PingError::AlreadyProcessed);
            }
            if state.received_ids.contains_key(&delivery.application_id) {
                return Err(PingError::AlreadyReceived);
            }
            state.received_ids.insert(delivery.application_id, key);
            state.receipts.insert(key, delivery.clone());
        }
        self.emit_event(Event::ReceiptSubmitted {
            slot,
            transaction_index,
            application_id: delivery.application_id,
            payload: delivery.payload.clone(),
        })
        .expect("Failed to emit receipt event");
        Ok(delivery)
    }

    #[export]
    pub fn received(&self, slot: u64, transaction_index: u64) -> Option<Delivery> {
        self.state
            .borrow()
            .receipts
            .get(&(slot, transaction_index))
            .cloned()
    }

    #[export]
    pub fn payload_of(&self, application_id: H256) -> Option<Vec<u8>> {
        let state = self.state.borrow();
        state
            .received_ids
            .get(&application_id)
            .and_then(|key| state.receipts.get(key))
            .map(|delivery| delivery.payload.clone())
    }

    #[export]
    pub async fn send_message(
        &mut self,
        application_id: H256,
        payload: Vec<u8>,
    ) -> Result<QueuedDelivery, PingError> {
        let (config, receiver) = {
            let state = self.state.borrow();
            if msg::source() != state.owner {
                return Err(PingError::NotOwner);
            }
            // A runtime error reply rolls back attached value before any dispatch/state effect.
            assert_eq!(
                msg::value(),
                state.bridge_config.fee_bridge,
                "Incorrect bridge fee"
            );
            validate_payload(application_id, payload.len())?;
            if state.outbound.contains_key(&application_id) {
                return Err(PingError::AlreadyRequested);
            }
            (state.bridge_config, state.ethereum_receiver)
        };
        let request_message_id = msg::id();
        self.state.borrow_mut().outbound.insert(
            application_id,
            OutboundRequest {
                request_message_id,
                builtin_message_id: None,
                reply_message_id: None,
                result: None,
            },
        );
        let mut wire_payload = Vec::with_capacity(32 + payload.len());
        wire_payload.extend_from_slice(application_id.as_bytes());
        wire_payload.extend_from_slice(&payload);
        let request = gbuiltin_eth_bridge::Request::SendEthMessage {
            destination: receiver,
            payload: wire_payload,
        }
        .encode();
        let reply = msg::send_bytes_with_gas_for_reply(
            config.builtin,
            request,
            config.gas_to_send_request_to_builtin,
            config.fee_bridge,
            config.gas_for_reply_deposit,
        )
        .map_err(|_| PingError::BuiltinFailure)?;
        let builtin_message_id = reply.waiting_reply_to;
        self.state
            .borrow_mut()
            .outbound
            .get_mut(&application_id)
            .expect("Retained outbound request")
            .builtin_message_id = Some(builtin_message_id);
        let state = self.state.clone();
        let awaited = reply
            .up_to(Some(config.reply_timeout))
            .map_err(|_| PingError::BuiltinFailure)?
            .handle_reply(move || {
                if msg::source() != config.builtin
                    || msg::reply_to().ok() != Some(builtin_message_id)
                {
                    return;
                }
                let result = match msg::reply_code() {
                    Ok(ReplyCode::Success(_)) => msg::load_bytes()
                        .map_err(|_| PingError::BuiltinDecode)
                        .and_then(|bytes| decode_builtin_reply(&bytes)),
                    _ => Err(PingError::BuiltinFailure),
                };
                let mut state = state.borrow_mut();
                let Some(original) = state.outbound.get_mut(&application_id) else {
                    return;
                };
                if original.request_message_id != request_message_id
                    || original.builtin_message_id != Some(builtin_message_id)
                    || original.reply_message_id.is_some()
                {
                    return;
                }
                original.reply_message_id = Some(msg::id());
                original.result = Some(result);
            })
            .map_err(|_| PingError::BuiltinFailure)?
            .await;
        let state = self.state.borrow();
        let original = state
            .outbound
            .get(&application_id)
            .expect("Retained outbound request");
        // Even after timeout, the request-bound reply hook can reconcile this original.
        // Never remove Pending or permit another dispatch for the same ID.
        original.result.clone().unwrap_or_else(|| {
            Err(if awaited.is_ok() {
                PingError::BuiltinDecode
            } else {
                PingError::BuiltinFailure
            })
        })
    }

    #[export]
    pub fn outbound(&self, application_id: H256) -> Option<OutboundStatus> {
        self.state
            .borrow()
            .outbound
            .get(&application_id)
            .map(|request| match &request.result {
                Some(Ok(delivery)) => OutboundStatus::Queued(delivery.clone()),
                _ => OutboundStatus::Pending,
            })
    }
}

fn validate_payload(application_id: H256, length: usize) -> Result<(), PingError> {
    if application_id.is_zero() || length > MAX_PAYLOAD {
        Err(PingError::InvalidPayload)
    } else {
        Ok(())
    }
}

fn decode_delivery(state: &State, receipt_rlp: &[u8]) -> Result<Delivery, PingError> {
    let mut bytes = receipt_rlp;
    let receipt = ReceiptEnvelope::decode(&mut bytes).map_err(|_| PingError::InvalidReceipt)?;
    // A post-state root is not an authenticated successful status on this lane.
    if !bytes.is_empty()
        || receipt
            .as_receipt()
            .and_then(|receipt| receipt.status.as_eip658())
            != Some(true)
    {
        return Err(PingError::InvalidReceipt);
    }
    let mut delivery = None;
    for log in receipt.logs() {
        if H160::from(log.address.0 .0) != state.ethereum_emitter
            || log.topics().first() != Some(&abi::MessageRequested::SIGNATURE_HASH)
        {
            continue;
        }
        let event = abi::MessageRequested::decode_raw_log_validate(log.topics(), &log.data.data)
            .map_err(|_| PingError::InvalidReceipt)?;
        // One dynamic bytes argument: reject noncanonical offsets, padding and trailing data.
        let data = &log.data.data;
        let padded_length = event.payload.len().div_ceil(32) * 32;
        if data.len() != 64 + padded_length
            || data[..31].iter().any(|byte| *byte != 0)
            || data[31] != 32
            || data[64 + event.payload.len()..]
                .iter()
                .any(|byte| *byte != 0)
        {
            return Err(PingError::InvalidReceipt);
        }
        if H160::from(event.sender.0 .0) != state.ethereum_sender {
            return Err(PingError::WrongSender);
        }
        if ActorId::from(event.destination.0) != gstd::exec::program_id() {
            return Err(PingError::WrongDestination);
        }
        let application_id = H256::from(event.applicationId.0);
        validate_payload(application_id, event.payload.len())?;
        if delivery.is_some() {
            return Err(PingError::AmbiguousDelivery);
        }
        delivery = Some(Delivery {
            application_id,
            payload: event.payload.to_vec(),
        });
    }
    delivery.ok_or(PingError::UnsupportedEvent)
}

fn decode_builtin_reply(mut bytes: &[u8]) -> Result<QueuedDelivery, PingError> {
    let response =
        gbuiltin_eth_bridge::Response::decode(&mut bytes).map_err(|_| PingError::BuiltinDecode)?;
    if !bytes.is_empty() {
        return Err(PingError::BuiltinDecode);
    }
    match response {
        gbuiltin_eth_bridge::Response::EthMessageQueued {
            block_number,
            hash,
            nonce,
            queue_id,
        } => Ok(QueuedDelivery {
            block_number,
            hash,
            nonce,
            queue_id,
        }),
    }
}

pub struct PingProgram {
    state: Rc<RefCell<State>>,
}

#[sails_rs::program]
impl PingProgram {
    pub fn new(
        historical_proxy: ActorId,
        ethereum_emitter: H160,
        ethereum_sender: H160,
        ethereum_receiver: H160,
        bridge_config: BridgeConfig,
    ) -> Self {
        assert!(
            !historical_proxy.is_zero() && !bridge_config.builtin.is_zero(),
            "Zero program identity"
        );
        assert!(
            !ethereum_emitter.is_zero()
                && !ethereum_sender.is_zero()
                && !ethereum_receiver.is_zero(),
            "Zero Ethereum identity"
        );
        assert_eq!(
            ethereum_emitter, ethereum_receiver,
            "Emitter and receiver must be the same MessageHandler"
        );
        assert!(
            bridge_config.gas_to_send_request_to_builtin > 0
                && bridge_config.gas_for_reply_deposit > 0
                && bridge_config.reply_timeout > 0,
            "Invalid bridge gas configuration"
        );
        assert_eq!(msg::value(), 0, "Initialization must not attach value");
        Self {
            state: Rc::new(RefCell::new(State {
                owner: msg::source(),
                historical_proxy,
                ethereum_emitter,
                ethereum_sender,
                ethereum_receiver,
                bridge_config,
                receipts: BTreeMap::new(),
                received_ids: BTreeMap::new(),
                outbound: BTreeMap::new(),
            })),
        }
    }

    pub fn ping(&self) -> PingService {
        PingService {
            state: self.state.clone(),
        }
    }
}
