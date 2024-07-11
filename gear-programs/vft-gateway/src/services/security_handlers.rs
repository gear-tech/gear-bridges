use super::{msg_tracker_mut, vft_master::vft_master::io as vft_master_io, MessageStatus, MessageTracker, MsgData, VftGateway};
use gstd::{msg};
use sails_rtl::{gstd::ExecContext, prelude::*};

impl<T> VftGateway<T>
where
    T: ExecContext,
{
    pub fn handle_reply() {
        let reply_to = msg::reply_to().expect("Unable to get the msg id");
        let mut msg_tracker = msg_tracker_mut();

        if let Some(msg_id) = msg_tracker.remove_waiting_reply(&reply_to) {
            let reply_bytes = msg::load_bytes().expect("Error during loading reply bytes");
            process_message_status(msg_id, reply_bytes, &mut msg_tracker);
        }
    }
}

fn process_message_status(
    msg_id: MessageId,
    reply_bytes: Vec<u8>,
    msg_tracker: &mut MessageTracker,
) {
    match msg_tracker
        .get_message_status_mut(&msg_id)
        .expect("Unexpected: msg status does not exist")
    {
        MessageStatus::SendingMessageToBurnTokens(_) => {
            handle_burn_tokens(reply_bytes, msg_id, msg_tracker)
        }
        MessageStatus::SendingMessageToBridgeBuiltin(_) => {
            handle_bridge_builtin(reply_bytes, msg_id, msg_tracker)
        }
        MessageStatus::SendingMessageToMintTokens(_) => {
            handle_mint_tokens(reply_bytes, msg_id, msg_tracker)
        }
        _ => unreachable!(),
    }
}

fn handle_burn_tokens(reply_bytes: Vec<u8>, msg_id: MessageId, msg_tracker: &mut MessageTracker) {
    match vft_master_io::Burn::decode_reply(&reply_bytes) {
        Ok(true) => {
            msg_tracker.update_message_status(msg_id, MessageStatus::TokenBurnCompleted(true))
        }
        _ => msg_tracker.update_message_status(msg_id, MessageStatus::TokenBurnCompleted(false)),
    }
}

fn handle_bridge_builtin(
    reply_bytes: Vec<u8>,
    msg_id: MessageId,
    msg_tracker: &mut MessageTracker,
) {
    match gbuiltin_bridge::Response::decode(&mut reply_bytes.as_slice()) {
        Ok(gbuiltin_bridge::Response::MessageSent { nonce, hash: _ }) => {
            msg_tracker
                .update_message_status(msg_id, MessageStatus::BridgeResponseReceived(true, nonce));
        }
        _ => {
            msg_tracker.update_message_status(
                msg_id,
                MessageStatus::BridgeResponseReceived(false, U256::zero()),
            );
        }
    }
}

fn handle_mint_tokens(reply_bytes: Vec<u8>, msg_id: MessageId, msg_tracker: &mut MessageTracker) {
    match vft_master_io::Mint::decode_reply(&reply_bytes) {
        Ok(true) => {
            msg_tracker.update_message_status(msg_id, MessageStatus::TokenMintCompleted(true))
        }
        _ => msg_tracker.update_message_status(msg_id, MessageStatus::TokenMintCompleted(false)),
    }
}

pub fn panic_handler(msg_tracker: &mut MessageTracker, msg_data: MsgData) {
    let erroneous_message_id = msg::signal_from().expect("Unable to get erroneous message id");
    let msg_status = msg_tracker
        .remove_message_status(&erroneous_message_id)
        .expect("Unexpected: msg status does not exist");
    match msg_status {
        MessageStatus::SendingMessageToBurnTokens(msg_id) => {
            msg_tracker.remove_waiting_reply(&msg_id);
        }
        MessageStatus::TokenBurnCompleted(false) => {}
        MessageStatus::TokenBurnCompleted(true)
        | MessageStatus::BridgeResponseReceived(_, _)
        | MessageStatus::TokenMintCompleted(_) => {
            msg_tracker.track_pending_message(erroneous_message_id, msg_status, msg_data);
        }
        MessageStatus::SendingMessageToBridgeBuiltin(msg_id)
        | MessageStatus::SendingMessageToMintTokens(msg_id) => {
            // For these statuses, track the message as pending to be resumed later

            msg_tracker.remove_waiting_reply(&msg_id);
            msg_tracker.track_pending_message(erroneous_message_id, msg_status, msg_data);
        }
    }
    msg_tracker.remove_message_status(&erroneous_message_id);
}
