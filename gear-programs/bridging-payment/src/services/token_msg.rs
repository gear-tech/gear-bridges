use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus, TransactionDetails},
    utils,
    vft::vft::io as vft_io,
    Config,
};

use sails_rs::{calls::ActionIo, prelude::*};

pub async fn transfer_tokens(
    token_id: ActorId,
    sender: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let msg_id = gstd::msg::id();
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    let transaction_details = TransactionDetails::Transfer {
        sender,
        receiver,
        amount,
        token_id,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToTransferTokens,
        transaction_details,
    );

    utils::set_critical_hook(msg_id);

    let reply_bytes = utils::send_message_with_gas_for_reply(
        token_id,
        bytes,
        config.gas_to_transfer_tokens,
        config.gas_for_reply_deposit,
    )
    .await?;

    let reply: bool = vft_io::TransferFrom::decode_reply(&reply_bytes)
        .map_err(|_| Error::TransferTokensDecodeError)?;

    if !reply {
        return Err(Error::ErrorDuringTokensTransfer);
    }
    Ok(())
}
