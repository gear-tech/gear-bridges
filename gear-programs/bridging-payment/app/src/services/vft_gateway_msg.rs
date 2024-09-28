use super::{error::Error, utils, Config};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;
use vft_gateway_client::{
    vft_gateway, vft_gateway::io as vft_gateway_io, Error as VftGatewayError,
};

pub async fn send_message_to_gateway(
    gateway_address: ActorId,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    config: &Config,
) -> Result<(U256, H160), Error> {
    let bytes: Vec<u8> =
        vft_gateway::io::TransferVaraToEth::encode_call(sender, vara_token_id, amount, receiver);

    let reply_bytes = utils::send_message_with_gas_for_reply(
        gateway_address,
        bytes,
        config.gas_to_send_request_to_gateway,
        config.gas_for_reply_deposit,
        config.reply_timeout,
    )
    .await?;

    Ok(decode_vft_gateway_reply(&reply_bytes)??)
}

fn decode_vft_gateway_reply(bytes: &[u8]) -> Result<Result<(U256, H160), VftGatewayError>, Error> {
    vft_gateway_io::TransferVaraToEth::decode_reply(bytes)
        .map_err(|_| Error::RequestToGateWayDecode)
}
