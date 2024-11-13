use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    BurnTokensDecode,
    TransferFromDecode,
    BuiltinDecode,
    MintTokensDecode,
    ReplyTimeout,
    NoCorrespondingEthAddress,
    ReplyHook,
    MessageNotFound,
    InvalidMessageStatus,
    MessageFailed,
    BurnTokensFailed,
    LockTokensFailed,
    BridgeBuiltinMessageFailed,
    TokensRefunded,
    NotEthClient,
    NotEnoughGas,
    NoCorrespondingVaraAddress,
    NotSupportedEvent,
}
