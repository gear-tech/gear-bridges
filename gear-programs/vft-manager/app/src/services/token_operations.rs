use super::msg_tracker::TxDetails;
use super::{msg_tracker_mut, utils, Config, Error, MessageStatus};
use vft_client::vft::io as vft_io;

use sails_rs::prelude::*;

pub async fn burn(
    vara_token_id: ActorId,
    sender: ActorId,
    receiver: H160,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_io::Burn::encode_call(sender, amount);

    let transaction_details = TxDetails::RequestBridging {
        vara_token_id,
        sender,
        amount,
        receiver,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToBurnTokens,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_to_burn_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;
    msg_tracker_mut().check_burn_result(&msg_id)
}

pub async fn mint(
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    msg_tracker_mut().update_message_status(msg_id, MessageStatus::SendingMessageToMintTokens);

    let bytes: Vec<u8> = vft_io::Mint::encode_call(receiver, amount);
    utils::send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_to_mint_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_mint_result(&msg_id)
}

pub async fn lock(
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    eth_receiver: H160,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let receiver = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    let transaction_details = TxDetails::RequestBridging {
        vara_token_id,
        sender,
        amount,
        receiver: eth_receiver,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToLockTokens,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_to_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_lock_result(&msg_id)
}

pub async fn unlock(
    vara_token_id: ActorId,
    recepient: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    msg_tracker_mut().update_message_status(msg_id, MessageStatus::SendingMessageToUnlockTokens);

    let sender = gstd::exec::program_id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, recepient, amount);
    utils::send_message_with_gas_for_reply(
        vara_token_id,
        bytes,
        config.gas_to_transfer_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_unlock_result(&msg_id)
}
