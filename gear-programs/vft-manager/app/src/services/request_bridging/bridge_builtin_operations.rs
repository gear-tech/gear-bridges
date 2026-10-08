//! Operations involving comunication with `pallet-gear-eth-bridge` built-in actor.

use gstd::{errors::Error as GStdError, msg, MessageId};
use sails_rs::prelude::*;

use super::{
    super::{Config, Error},
    msg_tracker::{msg_tracker_mut, MessageStatus},
};

/// Payload of the message that `ERC20Manager` will accept.
#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Payload {
    /// Account of the tokens sender.
    pub sender: ActorId,
    /// Account of the tokens receiver.
    pub receiver: H160,
    /// Address of the bridged `ERC20` token contract.
    pub token_id: H160,
    /// Bridged amount.
    pub amount: U256,
}

impl Payload {
    /// Pack [`Payload`] into a binary format that `ERC20Manager` will parse.
    pub fn pack(self) -> Vec<u8> {
        // ActorId is 32 bytes, H160 is 20 bytes (two fields), U256 is 32 bytes
        let mut packed = Vec::with_capacity(32 + 20 + 20 + 32);

        packed.extend_from_slice(self.sender.as_ref());
        packed.extend_from_slice(self.receiver.as_bytes());
        packed.extend_from_slice(self.token_id.as_bytes());

        let mut amount_bytes = [0u8; 32];
        self.amount.to_big_endian(&mut amount_bytes);
        packed.extend_from_slice(&amount_bytes);

        packed
    }
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum SourceRequestOutcome {
    Pending,
    Queued {
        nonce: U256,
        hash: H256,
        queue_id: u64,
    },
    NotQueued,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
pub struct SourceRequestEvidence {
    pub request: MessageId,
    pub child: MessageId,
    pub builtin: ActorId,
    pub request_hash: H256,
    pub outcome: SourceRequestOutcome,
}
static mut SOURCE_CHILDREN: Option<collections::BTreeMap<MessageId, MessageId>> = None;
fn children() -> &'static mut collections::BTreeMap<MessageId, MessageId> {
    unsafe { gstd::static_mut!(SOURCE_CHILDREN).get_or_insert_with(collections::BTreeMap::new) }
}
static mut SOURCE_REQUESTS: Option<collections::BTreeMap<MessageId, SourceRequestEvidence>> = None;
fn requests() -> &'static mut collections::BTreeMap<MessageId, SourceRequestEvidence> {
    unsafe { gstd::static_mut!(SOURCE_REQUESTS).get_or_insert_with(collections::BTreeMap::new) }
}
pub fn evidence(request: MessageId) -> Option<SourceRequestEvidence> {
    requests().get(&request).cloned()
}

/// Send bridging request to a `pallet-gear-eth-bridge` built-in actor.
///
/// It will asyncronously wait for reply from built-in and decode it
/// when it'll be received.
pub async fn send_message_to_bridge_builtin(
    gear_bridge_builtin: ActorId,
    erc20_manager: H160,
    payload: Payload,
    config: &Config,
    msg_id: MessageId,
) -> Result<(U256, H256, u64), Error> {
    let payload_bytes = payload.pack();
    let bytes = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: erc20_manager,
        payload: payload_bytes,
    }
    .encode();

    let future = gstd::msg::send_bytes_with_gas_for_reply(
        gear_bridge_builtin,
        &bytes,
        config.gas_to_send_request_to_builtin,
        config.fee_bridge,
        config.gas_for_reply_deposit,
    )
    .expect("Dispatch/reply funding failed: roll back the entire child dispatch");
    use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
    children().insert(future.waiting_reply_to, msg_id);
    requests().insert(
        msg_id,
        SourceRequestEvidence {
            request: msg_id,
            child: future.waiting_reply_to,
            builtin: gear_bridge_builtin,
            request_hash: H256::from(KeccakHasher::hash(
                &(
                    Syscall::program_id(),
                    gear_bridge_builtin,
                    erc20_manager,
                    &bytes,
                )
                    .encode(),
            )),
            outcome: SourceRequestOutcome::Pending,
        },
    );
    future
        .up_to(Some(config.reply_timeout))
        .expect("Cannot install child timeout: roll back dispatch")
        .handle_reply(handle_persistent_reply)
        .expect("Cannot install child reply hook: roll back dispatch")
        .await
        .map_err(|e| match e {
            GStdError::ErrorReply(..) => Error::MessageFailed,
            _ => Error::ReplyFailure(format!("{e:?}")),
        })?;

    if let Some(info) = msg_tracker_mut().get_message_info(&msg_id) {
        match info.status {
            MessageStatus::BridgeResponseReceived(Some((nonce, hash, queue_id))) => {
                msg_tracker_mut().remove_message_info(&msg_id);
                Ok((nonce, hash, queue_id))
            }
            MessageStatus::BridgeResponseReceived(None) => Err(Error::MessageFailed),
            _ => Err(Error::InvalidMessageStatus),
        }
    } else {
        Err(Error::MessageNotFound)
    }
}

/// Handle reply received from `pallet-gear-eth-bridge` built-in actor.
///
/// It will switch state of the currently processed message in
/// [message tracker](super::msg_tracker::MessageTracker) correspondingly.
pub fn handle_persistent_reply() {
    let Ok(child) = msg::reply_to() else { return };
    let Some(request) = children().get(&child).copied() else {
        return;
    };
    let evidence = requests().get(&request).unwrap();
    if evidence.builtin != msg::source() || evidence.outcome != SourceRequestOutcome::Pending {
        return;
    }
    let outcome = match msg::reply_code() {
        Ok(ReplyCode::Error(_)) => SourceRequestOutcome::NotQueued,
        Ok(ReplyCode::Success(_)) => match msg::load_bytes()
            .ok()
            .and_then(|bytes| decode_bridge_reply(&bytes).ok())
        {
            Some((nonce, hash, queue_id)) => SourceRequestOutcome::Queued {
                nonce,
                hash,
                queue_id,
            },
            None => return,
        },
        _ => return,
    };
    requests().get_mut(&request).unwrap().outcome = outcome.clone();
    if msg_tracker_mut()
        .get_message_info(&request)
        .is_some_and(|r| r.status == MessageStatus::SendingMessageToBridgeBuiltin)
    {
        let status = match outcome {
            SourceRequestOutcome::Queued {
                nonce,
                hash,
                queue_id,
            } => MessageStatus::BridgeResponseReceived(Some((nonce, hash, queue_id))),
            SourceRequestOutcome::NotQueued => MessageStatus::BridgeResponseReceived(None),
            SourceRequestOutcome::Pending => return,
        };
        msg_tracker_mut().update_message_status(request, status);
    }
}

pub fn reconcile(
    request: MessageId,
    child: MessageId,
    request_hash: H256,
) -> Result<SourceRequestOutcome, Error> {
    let evidence = evidence(request).ok_or(Error::InvalidReconciliation)?;
    if evidence.child != child
        || evidence.request_hash != request_hash
        || evidence.outcome == SourceRequestOutcome::Pending
    {
        return Err(Error::InvalidReconciliation);
    }
    let Some(info) = msg_tracker_mut().get_message_info(&request) else {
        return if matches!(evidence.outcome, SourceRequestOutcome::Queued { .. }) {
            Ok(evidence.outcome)
        } else {
            Err(Error::MessageNotFound)
        };
    };
    match evidence.outcome.clone() {
        SourceRequestOutcome::Queued {
            nonce,
            hash,
            queue_id,
        } => {
            if !matches!(
                info.status,
                MessageStatus::SendingMessageToBridgeBuiltin
                    | MessageStatus::BridgeResponseReceived(Some(_))
            ) {
                return Err(Error::InvalidReconciliation);
            }
            msg_tracker_mut().update_message_status(
                request,
                MessageStatus::BridgeResponseReceived(Some((nonce, hash, queue_id))),
            );
        }
        SourceRequestOutcome::NotQueued => {
            if !matches!(
                info.status,
                MessageStatus::SendingMessageToBridgeBuiltin
                    | MessageStatus::BridgeResponseReceived(None)
                    | MessageStatus::SendingMessageToReturnTokens
                    | MessageStatus::TokensReturnComplete(_)
            ) {
                return Err(Error::InvalidReconciliation);
            }
            if matches!(info.status, MessageStatus::SendingMessageToBridgeBuiltin) {
                msg_tracker_mut()
                    .update_message_status(request, MessageStatus::BridgeResponseReceived(None));
            }
        }
        SourceRequestOutcome::Pending => return Err(Error::InvalidReconciliation),
    }
    Ok(evidence.outcome)
}

/// Decode reply received from `pallet-gear-eth-bridge` built-in actor.
fn decode_bridge_reply(mut bytes: &[u8]) -> Result<(U256, H256, u64), Error> {
    let reply = gbuiltin_eth_bridge::Response::decode(&mut bytes)
        .map_err(|e| Error::BuiltinDecode(format!("{e:?}")))?;

    if !bytes.is_empty() {
        return Err(Error::BuiltinDecode("Trailing builtin reply".into()));
    }
    match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued {
            nonce,
            hash,
            queue_id,
            ..
        } => Ok((nonce, hash, queue_id)),
    }
}
