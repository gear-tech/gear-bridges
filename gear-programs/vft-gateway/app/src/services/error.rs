use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    BurnTokensDecode,
    BurnFailure,
    RequestToBuiltinSend,
    RequestToBuiltinReply,
    BuiltinDecode,
    PayloadSize,
    MintTokensDecode,
    ReplyTimeout,
    MintFailure,
    NoCorrespondingEthAddress,
    ReplyHook,
    MessageNotFound,
    InvalidMessageStatus,
    MessageFailed,
    BurnTokensFailed,
    BridgeBuiltinMessageFailed,
    TokensRefunded,
    NotEthClient,
    NotEnoughGas,
    NoCorrespondingVaraAddress,
}
