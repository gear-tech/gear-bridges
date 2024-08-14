use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus, TransactionDetails},
    utils,
    vft::vft::io as vft_io,
    Config,
};

use sails_rs::{calls::ActionIo, prelude::*};

pub async fn transfer_tokens(
    token_id: ActorId,
    sender: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let msg_id = gstd::msg::id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    let transaction_details = TransactionDetails::Transfer {
        sender,
        receiver,
        amount,
        token_id,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTransferTokens,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);

    utils::send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_to_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_transfer_result(&msg_id)
}

pub async fn transfer_tokens_back(
    token_id: ActorId,
    sender: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let msg_id = gstd::msg::id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    let transaction_details = TransactionDetails::Transfer {
        sender,
        receiver,
        amount,
        token_id,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTransferTokensBack,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);

    utils::send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_to_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_transfer_back_result(&msg_id)
}
