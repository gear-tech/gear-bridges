use super::{error::Error, msg_tracker_mut, MessageStatus};
use extended_vft_client::vft::io as vft_io;
use gstd::{msg, MessageId};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;

pub fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg info does not exist");

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

            MessageStatus::SendingMessageToLockTokens => {
                // If still sending, transition to `WaitingReplyFromLock`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromLock);
            }
            MessageStatus::TokenLockCompleted(true) => {
                // If the token transfer is successful, continue to bridge builtin step.
                msg_tracker.update_message_status(msg_id, MessageStatus::BridgeBuiltinStep);
            }
            MessageStatus::TokenLockCompleted(false) => {
                // If the token lock fails, cancel the transaction.
                msg_tracker.remove_message_info(&msg_id);
            }

            MessageStatus::SendingMessageToBridgeBuiltin => {
                // If still sending, transition to `WaitingReplyFromBuiltin`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromBuiltin);
            }
            MessageStatus::BridgeResponseReceived(None) => {
                // If error occurs during builtin message, go to mint step
                msg_tracker.update_message_status(msg_id, MessageStatus::MintTokensStep)
            }

            MessageStatus::SendingMessageToMintTokens => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromMint);
            }

            MessageStatus::SendingMessageToUnlockTokens => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromUnlock);
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
    msg_id: MessageId,
) -> Result<(), Error> {
    gstd::msg::send_bytes_with_gas_for_reply(destination, message, gas_to_send, 0, gas_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .handle_reply(move || handle_reply_hook(msg_id))
        .map_err(|_| Error::ReplyHook)?
        .await
        .map_err(|_| Error::ReplyFailure)?;
    Ok(())
}

fn handle_reply_hook(msg_id: MessageId) {
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg info does not exist");
    let reply_bytes = msg::load_bytes().expect("Unable to load bytes");

    match msg_info.status {
        MessageStatus::SendingMessageToBurnTokens => {
            match decode_burn_reply(&reply_bytes) {
                Ok(reply) => {
                    msg_tracker
                        .update_message_status(msg_id, MessageStatus::TokenBurnCompleted(reply));
                }
                Err(_) => {
                    msg_tracker.remove_message_info(&msg_id);
                }
            };
        }
        MessageStatus::WaitingReplyFromBurn => {
            let reply = decode_burn_reply(&reply_bytes).unwrap_or(false);
            if reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::BridgeBuiltinStep);
            } else {
                msg_tracker.remove_message_info(&msg_id);
            }
        }

        MessageStatus::SendingMessageToLockTokens => {
            match decode_lock_reply(&reply_bytes) {
                Ok(reply) => {
                    msg_tracker
                        .update_message_status(msg_id, MessageStatus::TokenLockCompleted(reply));
                }
                Err(_) => {
                    msg_tracker.remove_message_info(&msg_id);
                }
            };
        }
        MessageStatus::WaitingReplyFromLock => {
            let reply = decode_lock_reply(&reply_bytes).unwrap_or(false);
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
                    msg_tracker.update_message_status(msg_id, MessageStatus::MintTokensStep);
                }
            };
        }

        MessageStatus::WaitingReplyFromMint | MessageStatus::SendingMessageToMintTokens => {
            let reply = decode_mint_reply(&reply_bytes).unwrap_or(false);
            if !reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::MintTokensStep);
            } else {
                msg_tracker.update_message_status(msg_id, MessageStatus::TokenMintCompleted);
            }
        }

        MessageStatus::WaitingReplyFromUnlock | MessageStatus::SendingMessageToUnlockTokens => {
            let reply = decode_unlock_reply(&reply_bytes).unwrap_or(false);
            if !reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::UnlockTokensStep);
            } else {
                msg_tracker.update_message_status(msg_id, MessageStatus::TokenUnlockCompleted);
            }
        }
        _ => {}
    };
}

fn decode_burn_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Burn::decode_reply(bytes).map_err(|_| Error::BurnTokensDecode)
}

fn decode_lock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}

fn decode_bridge_reply(mut bytes: &[u8]) -> Result<Option<U256>, Error> {
    let reply =
        gbuiltin_eth_bridge::Response::decode(&mut bytes).map_err(|_| Error::BuiltinDecode)?;

    match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued { nonce, .. } => Ok(Some(nonce)),
    }
}

fn decode_mint_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Mint::decode_reply(bytes).map_err(|_| Error::MintTokensDecode)
}

fn decode_unlock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}
