use super::vft::vft::io as vft_io;
use sails_rs::{calls::ActionIo, prelude::*};

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    TransferTokensDecode,
    TokensTransferFailure,
    RequestToGateWayDecode,
    PayloadSize,
    MintTokensDecode,
    ReplyTimeout,
    TokensRefunded,
    TransactionFailure,
    FailureInVftGateway,
    ReplyHook,
    GatewayMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
    TransferTokensFailed,
}

pub async fn send_message_with_gas_for_reply(
    destination: ActorId,
    message: Vec<u8>,
    gas_to_send: u64,
    gas_deposit: u64,
    reply_timeout: u32,
) -> Result<Vec<u8>, Error> {
    gstd::msg::send_bytes_with_gas_for_reply(destination, message, gas_to_send, 0, gas_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .await
        .map_err(|_| Error::ReplyFailure)
}

pub async fn transfer_tokens(
    token_id: ActorId,
    sender: ActorId,
    receiver: ActorId,
    amount: U256,
) -> Result<bool, Error> {
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);
    let bytes =
        send_message_with_gas_for_reply(token_id, bytes, 15_000_000_000, 15_000_000_000, 1000)
            .await?;

    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferTokensDecode)
}
