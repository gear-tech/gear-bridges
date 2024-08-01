use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    SendError,
    ReplyError,
    BurnTokensDecodeError,
    ErrorDuringTokensBurn,
    RequestToBuiltinSendError,
    RequestToBuiltinReplyError,
    BuiltinDecodeError,
    PayloadSizeError,
    MintTokensDecodeError,
    ReplyTimeoutError,
    TokensRefundedError,
    ErrorDuringTokensMint,
}
