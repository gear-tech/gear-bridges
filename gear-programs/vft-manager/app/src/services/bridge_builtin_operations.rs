use super::{msg_tracker_mut, utils, Config, Error, MessageStatus};
use gstd::MessageId;
use sails_rs::prelude::*;

pub async fn send_message_to_bridge_builtin(
    gear_bridge_builtin: ActorId,
    receiver_contract_address: H160,
    payload: Payload,
    config: &Config,
    msg_id: MessageId,
) -> Result<U256, Error> {
    msg_tracker_mut().update_message_status(msg_id, MessageStatus::SendingMessageToBridgeBuiltin);

    let payload_bytes = payload.pack();

    let bytes = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: receiver_contract_address,
        payload: payload_bytes,
    }
    .encode();

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        gear_bridge_builtin,
        bytes,
        config.gas_to_send_request_to_builtin,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;
    msg_tracker_mut().check_bridge_reply(&msg_id)
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Payload {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

impl Payload {
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
