use gstd::{msg, MessageId};
use sails_rs::{calls::ActionIo, prelude::*};

use extended_vft_client::vft::io as vft_io;

use crate::services::TokenSupply;

use super::super::{Config, Error};
use super::msg_tracker::{msg_tracker_mut, MessageStatus, MessageTracker};

pub async fn burn(
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_io::Burn::encode_call(sender, amount);

    send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_for_token_ops,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    fetch_deposit_result(&*msg_tracker_mut(), &msg_id)
}

pub async fn lock(
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let receiver = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_for_token_ops,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    fetch_deposit_result(&*msg_tracker_mut(), &msg_id)
}

pub async fn mint(
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let msg_tracker = msg_tracker_mut();

    msg_tracker.update_message_status(msg_id, MessageStatus::SendingMessageToReturnTokens);

    let bytes: Vec<u8> = vft_io::Mint::encode_call(receiver, amount);
    send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_for_token_ops,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    fetch_withdraw_result(&*msg_tracker, &msg_id)
}

pub async fn unlock(
    vara_token_id: ActorId,
    recepient: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let msg_tracker = msg_tracker_mut();

    msg_tracker.update_message_status(msg_id, MessageStatus::SendingMessageToReturnTokens);

    let sender = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, recepient, amount);

    send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_for_token_ops,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    fetch_withdraw_result(&*msg_tracker, &msg_id)
}

fn fetch_deposit_result(msg_tracker: &MessageTracker, msg_id: &MessageId) -> Result<(), Error> {
    if let Some(info) = msg_tracker.message_info.get(msg_id) {
        match info.status {
            MessageStatus::TokenDepositCompleted(true) => Ok(()),
            MessageStatus::TokenDepositCompleted(false) => Err(Error::BurnTokensFailed),
            _ => Err(Error::InvalidMessageStatus),
        }
    } else {
        Err(Error::MessageNotFound)
    }
}

fn fetch_withdraw_result(msg_tracker: &MessageTracker, msg_id: &MessageId) -> Result<(), Error> {
    if let Some(info) = msg_tracker.message_info.get(msg_id) {
        match info.status {
            MessageStatus::TokensReturnComplete(true) => Ok(()),
            MessageStatus::TokensReturnComplete(false) => Err(Error::MessageFailed),
            _ => Err(Error::InvalidMessageStatus),
        }
    } else {
        Err(Error::MessageNotFound)
    }
}

async fn send_message_with_gas_for_reply(
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

fn decode_mint_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Mint::decode_reply(bytes).map_err(|_| Error::MintTokensDecode)
}

fn decode_unlock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}
