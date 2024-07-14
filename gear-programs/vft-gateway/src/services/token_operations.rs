use super::{vft_master::vft::io as vft_master_io, Config, Error, MessageStatus, MessageTracker};
use gstd::msg;
use sails::prelude::*;

pub async fn burn_tokens(
    token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
    msg_id: MessageId,
    msg_tracker: &mut MessageTracker,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_master_io::Burn::encode_call(sender, amount);

    let msg_future = msg::send_bytes_with_gas_for_reply(
        token_id.into(),
        bytes,
        config.gas_to_burn_tokens,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::BurnTokensSendError)?;

    // Store the ID of the sent message to understand which message we received a reply for in handle_reply
    let waiting_reply_to = msg_future.waiting_reply_to;
    msg_tracker.track_waiting_reply(waiting_reply_to, msg_id);
    msg_tracker.update_message_status(
        msg_id,
        MessageStatus::SendingMessageToBurnTokens(waiting_reply_to),
    );

    // At this moment, the message execution pauses and it enters the waitlist.
    let reply_bytes = msg_future.await.map_err(|_| Error::BurnTokensReplyError)?;

    let reply: bool = vft_master_io::Burn::decode_reply(&reply_bytes)
        .map_err(|_| Error::BurnTokensDecodeError)?;
    if !reply {
        return Err(Error::ErrorDuringTokensBurn);
    }
    Ok(())
}

pub async fn mint_tokens(
    token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let bytes: Vec<u8> = vft_master_io::Mint::encode_call(sender, amount);

    let reply_bytes = msg::send_bytes_with_gas_for_reply(
        token_id.into(),
        bytes,
        config.gas_to_mint_tokens,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::MintTokensSendError)?
    .await
    .map_err(|_| Error::MintTokensReplyError)?;

    let reply: bool = vft_master_io::Mint::decode_reply(&reply_bytes)
        .map_err(|_| Error::MintTokensDecodeError)?;
    if !reply {
        return Err(Error::ErrorDuringTokensMint);
    }
    Ok(())
}
