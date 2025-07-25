use sails_rs::prelude::*;

/// Error types for VFT Manageer service.
#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
pub enum Error {
    /// Error sending message to the program.
    SendFailure(String),
    /// Error while waiting for reply from the program.
    ReplyFailure(String),
    /// Failed to set reply timeout.
    ReplyTimeout(String),
    /// Failed to set reply hook.
    ReplyHook(String),
    /// A message does not have a reply code.
    NoReplyCode(String),

    /// Original `MessageId` wasn't found in message tracker when processing reply.
    MessageNotFound,
    /// Invalid message status was found in the message tracker when processing reply.
    InvalidMessageStatus,
    /// Message sent to the program failed.
    MessageFailed,

    /// Failed to decode Burn reply.
    BurnTokensDecode(String),
    /// Failed to decode TransferFrom reply.
    TransferFromDecode(String),
    /// Failed to decode Mint reply.
    MintTokensDecode(String),

    /// Failed to decode payload from gear-eth-bridge built-in actor.
    BuiltinDecode(String),
    /// Gas reservation for reply is too low.
    GasForReplyTooLow(String),

    /// `ERC20` address wasn't found in the token mapping.
    NoCorrespondingEthAddress,
    /// `VFT` address wasn't found in the token mapping.
    NoCorrespondingVaraAddress,

    /// `submit_receipt` can only be called by `historical-proxy` program.
    NotHistoricalProxy,

    /// Ethereum transaction receipt is not supported.
    UnsupportedEthEvent,
    /// Ethereum transaction is too old and already have been removed from storage.
    TransactionTooOld,
    /// Ethereum transaction was already processed by VFT Manager service.
    AlreadyProcessed,

    /// Vft-manager is paused and cannot process the request.
    Paused,

    /// Failed to burn tokens from the receiver in VftVara.
    BurnFromFailed(String),

    /// Internal unspecified VFT error
    Internal(String),
    /// Invalid or unexpected reply received from a VFT program.
    InvalidReply,
}
