use sails::prelude::*;

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
