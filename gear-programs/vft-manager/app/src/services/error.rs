use sails_rs::prelude::*;

/// Error types for VFT Manageer service.
#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    /// Error sending message to the program.
    SendFailure,
    /// Error while waiting for reply from the program.
    ReplyFailure,
    /// Failed to set reply timeout.
    ReplyTimeout,
    /// Failed to set reply hook.
    ReplyHook,

    /// Original `MessageId` wasn't found in message tracker when processing reply.
    MessageNotFound,
    /// Invalid message status was found in the message tracker when processing reply.
    InvalidMessageStatus,
    /// Message sent to the program failed.
    MessageFailed,

    /// Failed to decode `extended-vft::Burn` reply.
    BurnTokensDecode,
    /// Failed to decode `extended-vft::TransferFrom` reply.
    TransferFromDecode,
    /// Failed to decode `extended-vft::Mint` reply.
    MintTokensDecode,

    /// Failed to decode payload from gear-eth-bridge built-in actor.
    BuiltinDecode,

    /// `ERC20` address wasn't found in the token mapping.
    NoCorrespondingEthAddress,
    /// `VFT` address wasn't found in the token mapping.
    NoCorrespondingVaraAddress,

    /// `submit_receipt` can only be called by `historical-proxy` program.
    NotHistoricalProxy,

    /// Ethereum transaction receipt is not supported.
    NotSupportedEvent,
    /// Ethereum transaction is too old and already have been removed from storage.
    TransactionTooOld,
    /// Ethereum transaction was already processed by VFT Manager service.
    AlreadyProcessed,
}
