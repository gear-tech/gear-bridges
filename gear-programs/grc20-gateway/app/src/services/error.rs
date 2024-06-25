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