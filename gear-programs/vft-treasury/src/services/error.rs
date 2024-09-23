use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    BuiltinDecode,
    ReplyTimeout,
    DuplicateAddressMapping,
    NoCorrespondingEthAddress,
    ReplyHook,
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
