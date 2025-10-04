//! Operations involving comunication with `pallet-gear-eth-bridge` built-in actor.

use gstd::{msg, MessageId};
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
    let msg_tracker = msg_tracker_mut();

    let payload_bytes = payload.pack();
    let bytes = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: erc20_manager,
        payload: payload_bytes,
    }
    .encode();

    gstd::msg::send_bytes_with_gas_for_reply(
        gear_bridge_builtin,
        bytes,
        config.gas_to_send_request_to_builtin,
        config.fee_bridge,
        config.gas_for_reply_deposit,
    )
    .map_err(|e| Error::SendFailure(format!("{e:?}")))?
    .up_to(Some(config.reply_timeout))
    .map_err(|e| Error::ReplyTimeout(format!("{e:?}")))?
    .handle_reply(move || handle_reply_hook(msg_id))
    .map_err(|e| Error::ReplyHook(format!("{e:?}")))?
    .await
    .map_err(|e| Error::ReplyFailure(format!("{e:?}")))?;

    if let Some(info) = msg_tracker.get_message_info(&msg_id) {
        match info.status {
            MessageStatus::BridgeResponseReceived(Some((nonce, hash, queue_id))) => {
                msg_tracker.remove_message_info(&msg_id);
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
fn handle_reply_hook(msg_id: MessageId) {
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg info does not exist");
    let reply_bytes = msg::load_bytes().expect("Unable to load bytes");

    if msg_info.status == MessageStatus::SendingMessageToBridgeBuiltin {
        let reply = decode_bridge_reply(&reply_bytes).ok().flatten();
        msg_tracker.update_message_status(msg_id, MessageStatus::BridgeResponseReceived(reply));
    }
}

/// Decode reply received from `pallet-gear-eth-bridge` built-in actor.
fn decode_bridge_reply(mut bytes: &[u8]) -> Result<Option<(U256, H256, u64)>, Error> {
    let reply = gbuiltin_eth_bridge::Response::decode(&mut bytes)
        .map_err(|e| Error::BuiltinDecode(format!("{e:?}")))?;

    match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued {
            nonce,
            hash,
            queue_id,
            ..
        } => Ok(Some((nonce, hash, queue_id))),
    }
}
