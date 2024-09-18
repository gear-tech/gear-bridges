use super::Error;
use super::{msg_tracker::MessageStatus, msg_tracker_mut, vft::vft::io as vft_io};
use sails_rs::{calls::ActionIo, prelude::*};

pub fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg info does not exist");

        match msg_info.status {
            MessageStatus::SendingMessageToBridgeBuiltin => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromBuiltin);
            }

            MessageStatus::SendingMessageToTransferTokens => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromTransfer);
            }

            MessageStatus::TokenTransferCompleted(true) => {
                msg_tracker.update_message_status(msg_id, MessageStatus::BridgeBuiltinStep);
            }

            MessageStatus::TokenTransferCompleted(false) => {
                msg_tracker.remove_message_info(&msg_id);
            }

            MessageStatus::BridgeResponseReceived(None) => {}
            _ => {}
        }
    });
}

fn decode_transfer_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferTokensDecode)
}

fn decode_bridge_reply(mut bytes: &[u8]) -> Result<Option<U256>, Error> {
    let reply =
        gbuiltin_eth_bridge::Response::decode(&mut bytes).map_err(|_| Error::BuiltinDecode)?;

    match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued { nonce, .. } => Ok(Some(nonce)),
    }
}

fn handle_reply_hook(msg_id: MessageId) {
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg info does not exist");
    let reply_bytes = gstd::msg::load_bytes().expect("Unable to load bytes");

    match msg_info.status {
        MessageStatus::SendingMessageToTransferTokens => {
            match decode_transfer_reply(&reply_bytes) {
                Ok(reply) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::TokenTransferCompleted(reply),
                    );
                }

                Err(_) => {
                    msg_tracker.remove_message_info(&msg_id);
                }
            }
        }

        MessageStatus::WaitingReplyFromTransfer => {
            let reply = decode_transfer_reply(&reply_bytes).unwrap_or(false);

            if reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::BridgeBuiltinStep);
            } else {
                msg_tracker.remove_message_info(&msg_id);
            }
        }

        MessageStatus::SendingMessageToBridgeBuiltin => {
            let reply = decode_bridge_reply(&reply_bytes);

            let result = match reply {
                Ok(Some(nonce)) => Some(nonce),
                _ => None,
            };

            msg_tracker
                .update_message_status(msg_id, MessageStatus::BridgeResponseReceived(result));
        }

        MessageStatus::WaitingReplyFromBuiltin => {
            let reply = decode_bridge_reply(&reply_bytes);

            match reply {
                Ok(Some(nonce)) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::MessageProcessedWithSuccess(nonce),
                    );
                }

                _ => {
                    msg_tracker.remove_message_info(&msg_id);
                }
            }
        }

        _ => {}
    }
}

pub async fn send_message_with_gas_for_reply(
    destination: ActorId,
    message: Vec<u8>,
    gas_to_send: u64,
    gas_deposit: u64,
    reply_timeout: u32,
    msg_id: MessageId,
) -> Result<(), Error> {
    gstd::msg::send_bytes_with_gas_for_reply(destination, message, gas_to_send, 0, gas_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .handle_reply(move || handle_reply_hook(msg_id))
        .map_err(|_| Error::ReplyHook)?
        .await
        .map_err(|err| {
            gstd::debug!("reply failed with {:?}", err);
            Error::ReplyFailure
        })?;
    Ok(())
}
