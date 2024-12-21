use gstd::{msg, MessageId};
use sails_rs::{calls::ActionIo, prelude::*};

use extended_vft_client::vft::io as vft_io;

use crate::services::TokenSupply;

use super::super::{Config, Error};
use super::msg_tracker::{msg_tracker_mut, MessageStatus, MessageTracker};

/// Burn `amount` tokens from the `sender` address.
///
/// It will send `Burn` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
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

/// Transfer `amount` tokens from the `sender` address to the current program address,
/// effectively locking them.
///
/// It will send `TransferFrom` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
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

/// Mint `amount` tokens into the `receiver` address.
///
/// It will send `Mint` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
pub async fn mint(
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let msg_tracker = msg_tracker_mut();

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

/// Transfer `amount` tokens from the current program address to the current `receiver` address,
/// effectively unlocking them.
///
/// It will send `TransferFrom` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
pub async fn unlock(
    vara_token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let msg_tracker = msg_tracker_mut();

    let sender = gstd::exec::program_id();
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

    fetch_withdraw_result(&*msg_tracker, &msg_id)
}

/// Fetch result of the message sent to deposit tokens into this program.
///
/// It will look for the specified [MessageId] in the [MessageTracker] and return result
/// based on this message state. The state should be present in the [MessageTracker] according
/// to the [handle_reply_hook] logic.
fn fetch_deposit_result(msg_tracker: &MessageTracker, msg_id: &MessageId) -> Result<(), Error> {
    if let Some(info) = msg_tracker.message_info.get(msg_id) {
        match info.status {
            MessageStatus::TokenDepositCompleted(true) => Ok(()),
            MessageStatus::TokenDepositCompleted(false) => Err(Error::MessageFailed),
            _ => Err(Error::InvalidMessageStatus),
        }
    } else {
        Err(Error::MessageNotFound)
    }
}

/// Fetch result of the message sent to withdraw tokens from this program.
///
/// It will look for the specified [MessageId] in the [MessageTracker] and return result
/// based on this message state. The state should be present in the [MessageTracker] according
/// to the [handle_reply_hook] logic.
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

/// Configure parameters for message sending and send message
/// asyncronously waiting for the reply.
///
/// It will set reply hook to the [handle_reply_hook] and
/// timeout to the `reply_timeout`.
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

/// Handle reply received from `extended-vft` program.
///
/// It will drive [MessageTracker] state machine further.
fn handle_reply_hook(msg_id: MessageId) {
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg info does not exist");
    let reply_bytes = msg::load_bytes().expect("Unable to load bytes");

    match msg_info.status {
        MessageStatus::SendingMessageToDepositTokens => {
            let reply = match msg_info.details.token_supply {
                TokenSupply::Ethereum => decode_burn_reply(&reply_bytes),
                TokenSupply::Gear => decode_lock_reply(&reply_bytes),
            }
            .unwrap_or(false);

            msg_tracker.update_message_status(msg_id, MessageStatus::TokenDepositCompleted(reply));
        }
        MessageStatus::SendingMessageToReturnTokens => {
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

/// Decode reply received from the `extended-vft::Burn` method.
fn decode_burn_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Burn::decode_reply(bytes).map_err(|_| Error::BurnTokensDecode)
}

/// Decode reply received from the `extended-vft::TransferFrom` method.
fn decode_lock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}

/// Decode reply received from the `extended-vft::Mint` method.
fn decode_mint_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Mint::decode_reply(bytes).map_err(|_| Error::MintTokensDecode)
}

/// Decode reply received from the `extended-vft::TransferFrom` method.
fn decode_unlock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}
