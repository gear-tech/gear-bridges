use sails_rs::prelude::*;

use extended_vft_client::vft::io as vft_io;

use super::super::{Config, Error};
use super::msg_tracker::{msg_tracker_mut, MessageStatus, MessageTracker};
use super::utils;

pub async fn burn(
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_io::Burn::encode_call(sender, amount);

    utils::send_message_with_gas_for_reply(
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

    utils::send_message_with_gas_for_reply(
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
    utils::send_message_with_gas_for_reply(
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

    utils::send_message_with_gas_for_reply(
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
