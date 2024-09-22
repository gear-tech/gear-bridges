use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus, TransactionDetails},
    utils, vft_treasury, Config,
};
use sails_rs::prelude::*;

pub async fn send_message_to_treasury(
    treasury_address: ActorId,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    attached_value: u128,
    config: &Config,
) -> Result<(U256, H160), Error> {
    let msg_id = gstd::msg::id();

    let bytes: Vec<u8> = vft_treasury::vft_treasury::io::DepositTokens::encode_call(
        vara_token_id,
        sender,
        amount,
        receiver,
    );

    let transaction_details = TransactionDetails {
        sender,
        vara_token_id,
        amount,
        receiver,
        attached_value,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTreasury,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);

    utils::send_message_with_gas_for_reply(
        treasury_address,
        bytes,
        config.gas_to_send_request_to_treasury,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_vft_treasury_reply(&msg_id)
}
