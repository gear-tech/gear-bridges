use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    SendFailure(String),
    ReplyFailure(String),
    BuiltinDecode,
    ReplyTimeout,
    DuplicateAddressMapping,
    NoCorrespondingEthAddress,
    ReplyHook(String),
    MessageNotFound,
    InvalidMessageStatus,
    MessageFailed,
    BridgeBuiltinMessageFailed,
    TokensRefunded,
    NotEthClient,
    NotAdmin,
    NotBridgingClient,
    NotEnoughGas,
    TransferFailed,
    TransferTokensDecode,
}
