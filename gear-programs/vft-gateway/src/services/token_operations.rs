use super::msg_tracker::TransactionDetails;
use super::{msg_tracker_mut, utils, vft::vft::io as vft_io, Config, Error, MessageStatus};
#[allow(unused_imports)]
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;
pub async fn burn_tokens(
    token_id: ActorId,
    sender: ActorId,
    receiver: H160,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_io::Burn::encode_call(sender, amount);

    let transaction_details = TransactionDetails::new(sender, amount, receiver, token_id);

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToBurnTokens,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);
    utils::send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_to_burn_tokens,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;
    msg_tracker_mut().check_burn_result(&msg_id)
}

pub async fn mint_tokens(
    token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
) -> Result<(), Error> {
    msg_tracker_mut().update_message_status(msg_id, MessageStatus::SendingMessageToMintTokens);

    let bytes: Vec<u8> = vft_io::Mint::encode_call(sender, amount);

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
    //  msg_tracker_mut().remove_message_info(&msg_id);
    //   Ok(())
}
