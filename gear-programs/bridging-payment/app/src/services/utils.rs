use super::{grc20_gateway, Config};
use gstd::{msg, prelude::collections::HashMap, MessageId};
use sails_rtl::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    RequestToGateWaySendError,
    RequestToGateWayReplyError,
    RequestToGateWayDecodeError,
    RequestToBuiltinReplyError,
    PayloadSizeError,
    MintTokensSendError,
    MintTokensReplyError,
    MintTokensDecodeError,
    ReplyTimeoutError,
    TokensRefundedError,
}

pub async fn send_message_to_gateway(
    gateway_address: ActorId,
    sender: ActorId,
    amount: U256,
    receiver: [u8; 20],
    eth_token_id: [u8; 20],
    config: &Config,
) -> Result<Result<U256, grc20_gateway::Error>, Error> {
    let bytes: Vec<u8> = grc20_gateway::grc_20_gateway_io::TeleportVaraToEth::encode_call(
        sender,
        amount,
        receiver,
        eth_token_id,
    );

    let reply_bytes = msg::send_bytes_with_gas_for_reply(
        gateway_address.into(),
        bytes,
        config.gas_to_send_request_to_gateway,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::RequestToGateWaySendError)?
    .await
    .map_err(|_| Error::RequestToGateWayReplyError)?;

    let reply: Result<U256, grc20_gateway::Error> =
        grc20_gateway::grc_20_gateway_io::TeleportVaraToEth::decode_reply(&reply_bytes)
            .map_err(|_| Error::RequestToGateWayDecodeError)?;

    Ok(reply)
}
