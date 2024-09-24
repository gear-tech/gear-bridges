use super::{
    error::Error, vft_treasury::vft_treasury::io as vft_treasury_io,
    vft_treasury::Error as VftTreasuryError,
};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;

pub async fn send_message_with_gas_for_reply(
    destination: ActorId,
    message: Vec<u8>,
    gas_to_send: u64,
    gas_deposit: u64,
    reply_timeout: u32,
) -> Result<(U256, H160), Error> {
    let reply_bytes =
        gstd::msg::send_bytes_with_gas_for_reply(destination, message, gas_to_send, 0, gas_deposit)
            .map_err(|_| Error::SendFailure)?
            .up_to(Some(reply_timeout))
            .map_err(|_| Error::ReplyTimeout)?
            .await
            .map_err(|_| Error::ReplyFailure)?;

    let (nonce, eth_token_id) = decode_vft_treasury_reply(&reply_bytes)??;

    Ok((nonce, eth_token_id))
}

fn decode_vft_treasury_reply(
    bytes: &[u8],
) -> Result<Result<(U256, H160), VftTreasuryError>, Error> {
    vft_treasury_io::DepositTokens::decode_reply(bytes).map_err(|_| Error::RequestToTreasuryDecode)
}
