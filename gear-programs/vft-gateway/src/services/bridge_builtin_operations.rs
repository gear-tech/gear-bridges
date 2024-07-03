use super::{Config, Error, MessageStatus, MessageTracker};
use gstd::{msg, MessageId};
use sails_rtl::prelude::*;

pub async fn send_message_to_bridge_builtin(
    gear_bridge_builtin: ActorId,
    receiver: H160,
    token_id: H160,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
    msg_tracker: &mut MessageTracker,
) -> Result<U256, Error> {
    let payload_bytes = Payload {
        receiver,
        token_id,
        amount,
    }
    .pack();

    let request = gbuiltin_bridge::Request::SendMessage {
        dest: token_id,
        payload: payload_bytes
            .try_into()
            .map_err(|_| Error::PayloadSizeError)?,
    };

    let msg_future = msg::send_with_gas_for_reply_as(
        gear_bridge_builtin.into(),
        request,
        config.gas_to_send_request_to_builtin,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::RequestToBuiltinSendError)?
    .up_to(Some(config.reply_timeout))
    .map_err(|_| Error::ReplyTimeoutError)?;

    let waiting_reply_to = msg_future.waiting_reply_to;
    msg_tracker.track_waiting_reply(waiting_reply_to, msg_id);
    msg_tracker.update_message_status(
        msg_id,
        MessageStatus::SendingMessageToBridgeBuiltin(waiting_reply_to),
    );

    let reply: gbuiltin_bridge::Response = msg_future
        .await
        .map_err(|_| Error::RequestToBuiltinReplyError)?;

    match reply {
        gbuiltin_bridge::Response::MessageSent { nonce, hash: _ } => Ok(nonce),
        _ => Err(Error::RequestToBuiltinReplyError),
    }
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Payload {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

impl Payload {
    pub fn pack(self) -> Vec<u8> {
        let mut packed = Vec::with_capacity(20 + 20 + 32); // H160 is 20 bytes, U256 is 32 bytes

        packed.extend_from_slice(self.receiver.as_bytes());
        packed.extend_from_slice(self.token_id.as_bytes());

        let mut amount_bytes = [0u8; 32];
        self.amount.to_big_endian(&mut amount_bytes);
        packed.extend_from_slice(&amount_bytes);

        packed
    }
}
