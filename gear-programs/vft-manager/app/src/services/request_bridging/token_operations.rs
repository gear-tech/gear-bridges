use gstd::{msg, MessageId};
use sails_rs::{calls::ActionIo, prelude::*};

use vft_client::{
    vft::io::TransferFrom,
    vft_admin::io::{Burn, Mint},
};

use crate::services::TokenSupply;

use super::{
    super::{Config, Error},
    msg_tracker::{msg_tracker_mut, MessageStatus, MessageTracker},
};

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
    let bytes: Vec<u8> = Burn::encode_call(sender, amount);

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
    let receiver = Syscall::program_id();
    let bytes: Vec<u8> = TransferFrom::encode_call(sender, receiver, amount);

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

    let bytes: Vec<u8> = Mint::encode_call(receiver, amount);
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

/// Transfer `amount` tokens from the current program address to the `receiver` address,
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

    let sender = Syscall::program_id();
    let bytes: Vec<u8> = TransferFrom::encode_call(sender, receiver, amount);

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
        .map_err(|e| Error::SendFailure(format!("{e:?}")))?
        .up_to(Some(reply_timeout))
        .map_err(|e| Error::ReplyTimeout(format!("{e:?}")))?
        .handle_reply(move || handle_reply_hook(msg_id))
        .map_err(|e| Error::ReplyHook(format!("{e:?}")))?
        .await
        .map_err(|e| Error::ReplyFailure(format!("{e:?}")))?;

    Ok(())
}

/// Handle reply received from `VFT` program.
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

/// Decode reply received from the Burn method.
fn decode_burn_reply(bytes: &[u8]) -> Result<bool, Error> {
    Burn::decode_reply(bytes)
        .map_err(|e| Error::BurnTokensDecode(format!("{e:?}")))
        .map(|_| true)
}

/// Decode reply received from the TransferFrom method.
fn decode_lock_reply(bytes: &[u8]) -> Result<bool, Error> {
    TransferFrom::decode_reply(bytes).map_err(|e| Error::TransferFromDecode(format!("{e:?}")))
}

/// Decode reply received from the Mint method.
fn decode_mint_reply(bytes: &[u8]) -> Result<bool, Error> {
    Mint::decode_reply(bytes)
        .map_err(|e| Error::MintTokensDecode(format!("{e:?}")))
        .map(|_| true)
}

/// Decode reply received from the TransferFrom method.
fn decode_unlock_reply(bytes: &[u8]) -> Result<bool, Error> {
    TransferFrom::decode_reply(bytes).map_err(|e| Error::TransferFromDecode(format!("{e:?}")))
}
