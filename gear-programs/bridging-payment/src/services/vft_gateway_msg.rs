use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus, TransactionDetails},
    utils, vft_gateway,
    vft_gateway::vft_gateway::io as vft_gateway_io,
    Config,
};
use gstd::{msg, prelude::collections::HashMap, MessageId};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;

pub async fn send_message_to_gateway(
    gateway_address: ActorId,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    attached_value: u128,
    config: &Config,
) -> Result<(U256, H160), Error> {
    let msg_id = gstd::msg::id();

    let bytes: Vec<u8> = vft_gateway::vft_gateway::io::TransferVaraToEth::encode_call(
        vara_token_id,
        amount,
        receiver,
    );

    let transaction_details = TransactionDetails::SendMessageToGateway {
        sender,
        vara_token_id,
        amount,
        receiver,
        attached_value,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToGateway,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);

    utils::send_message_with_gas_for_reply(
        gateway_address,
        bytes,
        config.gas_to_send_request_to_gateway,
        config.gas_for_reply_deposit,
        config.reply_timeout,
        msg_id,
    )
    .await?;

    msg_tracker_mut().check_vft_gateway_reply(&msg_id)
}
