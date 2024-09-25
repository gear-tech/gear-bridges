use super::msg_tracker::MessageStatus;
use super::msg_tracker::TxDetails;
use super::msg_tracker_mut;
use super::utils;

use super::vft::vft::io as vft_io;
use super::Config;
use super::Error;
use gstd::{ActorId, MessageId};
use sails_rs::prelude::*;

/// Deposit VFT of `vara_token_id` to treasury from `sender` with `amount` of tokens
/// expecting it to arrive to `receiver` on Ethereum, `eth_token_id` is a contract which
/// implements the token on ETH network.
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

    let transaction_detail = TxDetails::DepositToTreasury {
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

/// Withdraw `vara_token_id` of `amount` from treasury to `recepient` account. It is expected that someone
/// burned the necessary `amount` of tokens on Ethereum network and then ethereum event client send the
/// event to us to perfrorm a withdraw transaction.
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

    let transaction_detail = TxDetails::WithdrawFromTreasury {
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
