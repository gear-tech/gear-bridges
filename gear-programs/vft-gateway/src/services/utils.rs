use super::{error::Error, msg_tracker_mut, MessageStatus};
use sails_rs::{prelude::*};

pub fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg status does not exist");

        match msg_info.status {
            MessageStatus::SendingMessageToBurnTokens => {
                // If still sending, transition to `WaitingReplyFromBurn`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromBurn);
            }
            MessageStatus::TokenBurnCompleted(true) => {
                // If the token transfer is successful, continue to bridge builtin step.
                msg_tracker.update_message_status(msg_id, MessageStatus::BridgeBuiltinStep);
            }
            MessageStatus::TokenBurnCompleted(false) => {
                // If the token burn fails, cancel the transaction.
                msg_tracker.remove_message_info(&msg_id);
            }
            MessageStatus::SendingMessageToBridgeBuiltin => {
                // If still sending, transition to `WaitingReplyFromBurn`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromBuiltin);
            }
            MessageStatus::BridgeResponseReceived(None) => {
                // If error occurs during builtin message, go to mint step
                msg_tracker.update_message_status(msg_id, MessageStatus::MintTokensStep)
            }
            MessageStatus::SendingMessageToMintTokens => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromMint);
            }
            MessageStatus::TokenMintCompleted(false) => {
                // retry
                msg_tracker.update_message_status(msg_id, MessageStatus::MintTokensStep)
            }
            _ => {}
        };
    });
}

pub async fn send_message_with_gas_for_reply(
    destination: ActorId,
    message: Vec<u8>,
    gas_to_send: u64,
    gas_deposit: u64,
    reply_timeout: u32,
) -> Result<Vec<u8>, Error> {
    gstd::msg::send_bytes_with_gas_for_reply(
        destination.into(),
        message,
        gas_to_send,
        0,
        gas_deposit,
    )
    .map_err(|_| Error::SendError)?
    .up_to(Some(reply_timeout))
    .map_err(|_| Error::ReplyTimeoutError)?
    .await
    .map_err(|_| Error::ReplyError)
}

