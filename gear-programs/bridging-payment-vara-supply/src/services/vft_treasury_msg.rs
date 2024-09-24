use super::{error::Error, utils, vft_treasury, Config};
use sails_rs::prelude::*;

pub async fn send_message_to_treasury(
    treasury_address: ActorId,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    config: &Config,
) -> Result<(U256, H160), Error> {
    let bytes: Vec<u8> = vft_treasury::vft_treasury::io::DepositTokens::encode_call(
        vara_token_id,
        sender,
        amount,
        receiver,
    );

    utils::send_message_with_gas_for_reply(
        treasury_address,
        bytes,
        config.gas_to_send_request_to_treasury,
        config.gas_for_reply_deposit,
        config.reply_timeout,
    )
    .await
}
