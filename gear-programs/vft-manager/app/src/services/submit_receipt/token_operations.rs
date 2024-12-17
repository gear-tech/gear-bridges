use gstd::msg;
use sails_rs::{calls::ActionIo, prelude::*};

use extended_vft_client::vft::io as vft_io;

use super::{
    super::{Config, Error, TokenSupply},
    msg_tracker::{msg_tracker_mut, MessageStatus},
};

pub async fn mint(
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_io::Mint::encode_call(receiver, amount);
    send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_for_token_ops,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await
}

pub async fn unlock(
    vara_token_id: ActorId,
    recepient: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
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
    .await
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
        MessageStatus::SendingMessageToWithdrawTokens
        | MessageStatus::WaitingReplyFromTokenWithdrawMessage => {
            let reply = match msg_info.details.token_supply {
                TokenSupply::Ethereum => decode_mint_reply(&reply_bytes),
                TokenSupply::Gear => decode_unlock_reply(&reply_bytes),
            }
            .unwrap_or(false);

            if !reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::TokenWithdrawFailed);
            } else {
                msg_tracker.update_message_status(msg_id, MessageStatus::TokenWithdrawCompleted);
            }
        }

        _ => {}
    };
}

fn decode_mint_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::Mint::decode_reply(bytes).map_err(|_| Error::MintTokensDecode)
}

fn decode_unlock_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferFromDecode)
}
