use super::msg_tracker::MessageStatus;
use super::msg_tracker::TxDetails;
use super::msg_tracker_mut;
use super::utils;
/*
pub async fn transfer_tokens(
    token_id: ActorId,
    sender: ActorId,
    receiver: ActorId,
    amount: U256,
) -> Result<bool, Error> {
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);
    let bytes =
        send_message_with_gas_for_reply(token_id, bytes, 15_000_000_000, 15_000_000_000, 1000)
            .await?;

    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferTokensDecode)
}
 */
use super::vft::vft::io as vft_io;
use super::Config;
use super::Error;
use gstd::{ActorId, MessageId};
use sails_rs::prelude::*;

pub async fn deposit_to_treasury(
    vara_token_id: ActorId,
    eth_token_id: H160,
    sender: ActorId,
    amount: U256,
    eth_receiver: H160,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let receiver = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    let transaction_detail = TxDetails::DepositVaraToTreasury {
        vara_token_id,
        eth_token_id,
        sender,
        amount,
        receiver: eth_receiver,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTransferTokens,
        transaction_detail,
    );

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_for_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;
    msg_tracker_mut().check_transfer_result(&msg_id)
}

pub async fn withdraw_from_treasury(
    vara_token_id: ActorId,
    eth_token_id: H160,
    recepient: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let sender = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, recepient, amount);

    let transaction_detail = TxDetails::WithdrawVaraFromTreasury {
        vara_token_id,
        eth_token_id,
        recepient,
        amount,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTransferTokens,
        transaction_detail,
    );

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_for_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;
    msg_tracker_mut().check_transfer_result(&msg_id)
}
