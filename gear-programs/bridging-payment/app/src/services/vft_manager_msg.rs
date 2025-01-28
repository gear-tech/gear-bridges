use super::{error::Error, Config};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;
use vft_manager_client::vft_manager::io as vft_manager_io;

/// Send bridging request to the `vft-manager`.
pub async fn send(
    vft_manager_address: ActorId,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    config: &Config,
) -> Result<(U256, H160), Error> {
    let bytes: Vec<u8> =
        vft_manager_io::RequestBridgingPayed::encode_call(sender, vara_token_id, amount, receiver);

    let reply_bytes = gstd::msg::send_bytes_with_gas_for_reply(
        vft_manager_address,
        bytes,
        config.gas_to_send_request_to_vft_manager,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::SendFailure)?
    .up_to(Some(config.reply_timeout))
    .map_err(|_| Error::ReplyTimeout)?
    .await
    .map_err(|_| Error::ReplyFailure)?;

    Ok(vft_manager_io::RequestBridging::decode_reply(&reply_bytes)
        .map_err(|_| Error::RequestToVftManagerDecode)??)
}
