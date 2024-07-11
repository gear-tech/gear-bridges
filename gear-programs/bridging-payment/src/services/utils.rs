use super::{vft_gateway::vft_gateway::io as vft_gateway_io, vft_gateway, Config};
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
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    config: &Config,
) -> Result<Result<(U256, H160), vft_gateway::Error>, Error> {
    let bytes: Vec<u8> = vft_gateway::vft_gateway::io::TransferVaraToEth::encode_call(
        sender,
        vara_token_id,
        amount,
        receiver,
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

    let reply: Result<(U256, H160), vft_gateway::Error> =
        vft_gateway_io::TransferVaraToEth::decode_reply(&reply_bytes)
            .map_err(|_| Error::RequestToGateWayDecodeError)?;

    Ok(reply)
}
