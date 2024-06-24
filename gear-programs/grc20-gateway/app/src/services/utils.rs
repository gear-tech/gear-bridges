use super::{erc20, Config};
use gstd::{msg, prelude::collections::HashMap, MessageId};
use sails_rtl::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    BurnTokensSendError,
    BurnTokensReplyError,
    BurnTokensDecodeError,
    ErrorDuringTokensBurn,
    RequestToBuiltinSendError,
    RequestToBuiltinReplyError,
    PayloadSizeError,
    MintTokensSendError,
    MintTokensReplyError,
    MintTokensDecodeError,
    ReplyTimeoutError,
    TokensRefundedError,
    ErrorDuringTokensMint,
}

pub async fn burn_tokens(
    token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let bytes: Vec<u8> = erc20::admin_io::Burn::encode_call(sender, amount);

    let reply_bytes = msg::send_bytes_with_gas_for_reply(
        token_id.into(),
        bytes,
        config.gas_to_burn_tokens,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::BurnTokensSendError)?
    .await
    .map_err(|_| Error::BurnTokensReplyError)?;

    let reply: bool = erc20::admin_io::Burn::decode_reply(&reply_bytes)
        .map_err(|_| Error::BurnTokensDecodeError)?;
    if !reply {
        return Err(Error::ErrorDuringTokensBurn);
    }
    Ok(())
}

pub async fn send_message_to_bridge_builtin(
    gear_bridge_builtin: ActorId,
    receiver: H160,
    token_id: H160,
    amount: U256,
    config: &Config,
) -> Result<U256, Error> {
    let payload_bytes = Payload {
        receiver,
        token_id,
        amount,
    }
    .pack();

    let request = gbuiltin_bridge::Request::SendMessage {
        dest: token_id,
        payload: payload_bytes
            .try_into()
            .map_err(|_| Error::PayloadSizeError)?,
    };

    let reply: gbuiltin_bridge::Response = msg::send_with_gas_for_reply_as(
        gear_bridge_builtin.into(),
        request,
        config.gas_to_send_request_to_builtin,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|_| Error::RequestToBuiltinSendError)?
    .up_to(Some(config.reply_timeout))
    .map_err(|_| Error::ReplyTimeoutError)?
    .await
    .map_err(|_| Error::RequestToBuiltinReplyError)?;

    match reply {
        gbuiltin_bridge::Response::MessageSent { nonce, hash: _ } => Ok(nonce),
        _ => Err(Error::RequestToBuiltinReplyError),
    }
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Payload {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

impl Payload {
    pub fn pack(self) -> Vec<u8> {
        let mut packed = Vec::with_capacity(20 + 20 + 32); // H160 is 20 bytes, U256 is 32 bytes

        packed.extend_from_slice(self.receiver.as_bytes());
        packed.extend_from_slice(self.token_id.as_bytes());

        let mut amount_bytes = [0u8; 32];
        self.amount.to_big_endian(&mut amount_bytes);
        packed.extend_from_slice(&amount_bytes);

        packed
    }
}

pub async fn mint_tokens(
    token_id: ActorId,
    sender: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let bytes: Vec<u8> = erc20::admin_io::Mint::encode_call(sender, amount);

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

    let reply: bool = erc20::admin_io::Mint::decode_reply(&reply_bytes)
        .map_err(|_| Error::MintTokensDecodeError)?;
    if !reply {
        return Err(Error::ErrorDuringTokensMint);
    }
    Ok(())
}

struct MessageTracker {
    msg_id_to_payload: HashMap<MessageId, MessageStatus>,
    waiting_reply_to_msg_id: HashMap<MessageId, MessageId>,
}

#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    SendingMessageToBurnTokens,
    TokenBurnCompleted(bool),
    SendingMessageToBridgeBuiltin,
    BridgeResponseReceived(bool),
}
// #[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
// pub enum ActionToComplete {
//     CompleteEthToVaraBatch {
//         teleports: Vec<EthToVaraTransfer>,
//         eth_to_vara_nonce: U256,
//         last_executed_eth_nonce_id: U256,
//     },
//     CompleteVaraToEth {
//         sender: ActorId,
//         recipient: String,
//         amount: U256,
//     },
// }
