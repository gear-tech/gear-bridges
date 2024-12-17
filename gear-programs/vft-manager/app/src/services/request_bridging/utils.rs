use super::super::error::Error;
use super::msg_tracker::{msg_tracker_mut, MessageStatus};
use super::TokenSupply;

use extended_vft_client::vft::io as vft_io;
use gstd::{msg, MessageId};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;

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
        MessageStatus::SendingMessageToDepositTokens
        | MessageStatus::WaitingReplyFromTokenDepositMessage => {
            let reply = match msg_info.details.token_supply {
                TokenSupply::Ethereum => decode_burn_reply(&reply_bytes),
                TokenSupply::Gear => decode_lock_reply(&reply_bytes),
            }
            .unwrap_or(false);

            msg_tracker.update_message_status(msg_id, MessageStatus::TokenDepositCompleted(reply));
        }
        MessageStatus::SendingMessageToBridgeBuiltin | MessageStatus::WaitingReplyFromBuiltin => {
            let reply = decode_bridge_reply(&reply_bytes).ok().flatten();
            msg_tracker.update_message_status(msg_id, MessageStatus::BridgeResponseReceived(reply));
        }
        MessageStatus::WaitingReplyFromTokenReturnMessage
        | MessageStatus::SendingMessageToReturnTokens => {
            let reply = match msg_info.details.token_supply {
                TokenSupply::Ethereum => decode_mint_reply(&reply_bytes),
                TokenSupply::Gear => decode_unlock_reply(&reply_bytes),
            }
            .unwrap_or(false);

            msg_tracker.update_message_status(msg_id, MessageStatus::TokensReturnComplete(reply));
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
