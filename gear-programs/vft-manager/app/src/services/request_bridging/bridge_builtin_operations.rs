//! Operations involving comunication with `pallet-gear-eth-bridge` built-in actor.

use gstd::MessageId;
use sails_rs::prelude::*;

use super::{
    super::{Config, Error},
    msg_tracker::{msg_tracker_mut, MessageStatus},
    utils,
};

/// Send bridging request to a `pallet-gear-eth-bridge` built-in actor.
pub async fn send_message_to_bridge_builtin(
    gear_bridge_builtin: ActorId,
    erc20_manager: H160,
    payload: Payload,
    config: &Config,
    msg_id: MessageId,
) -> Result<U256, Error> {
    let msg_tracker = msg_tracker_mut();

    msg_tracker.update_message_status(msg_id, MessageStatus::SendingMessageToBridgeBuiltin);

    let payload_bytes = payload.pack();

    let bytes = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: erc20_manager,
        payload: payload_bytes,
    }
    .encode();

    utils::send_message_with_gas_for_reply(
        gear_bridge_builtin,
        bytes,
        config.gas_to_send_request_to_builtin,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    if let Some(info) = msg_tracker.get_message_info(&msg_id) {
        match info.status {
            MessageStatus::BridgeResponseReceived(Some(nonce)) => {
                msg_tracker.remove_message_info(&msg_id);
                Ok(nonce)
            }
            MessageStatus::BridgeResponseReceived(None) => Err(Error::BridgeBuiltinMessageFailed),
            _ => Err(Error::InvalidMessageStatus),
        }
    } else {
        Err(Error::MessageNotFound)
    }
}

/// Payload of the message that `ERC20Manager` will accept.
#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Payload {
    /// Account of the tokens receiver.
    pub receiver: H160,
    /// Address of the bridged `ERC20` token contract.
    pub token_id: H160,
    /// Bridged amount.
    pub amount: U256,
}

impl Payload {
    /// Pack [Payload] into a binary format that `ERC20Manager` will parse.
    pub fn pack(self) -> Vec<u8> {
        // H160 is 20 bytes, U256 is 32 bytes
        let mut packed = Vec::with_capacity(20 + 20 + 32);

        packed.extend_from_slice(self.receiver.as_bytes());
        packed.extend_from_slice(self.token_id.as_bytes());

        let mut amount_bytes = [0u8; 32];
        self.amount.to_big_endian(&mut amount_bytes);
        packed.extend_from_slice(&amount_bytes);

        packed
    }
}
