use sails_rs::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
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
    ErrorDuringTokensMint,
    NoCorrespondingEthAddress,
    ReplyHook,
    MessageNotFound,
    InvalidMessageStatus,
    MessageFailed,
    BurnTokensFailed,
    BridgeBuiltinMessageFailed,
    TokensRefundedError,
}
