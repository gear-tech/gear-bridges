use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
pub enum Error {
    SendError,
    ReplyError,
    TransferTokensDecodeError,
    ErrorDuringTokensTransfer,
    RequestToGateWayDecodeError,
    PayloadSizeError,
    MintTokensSendError,
    MintTokensReplyError,
    MintTokensDecodeError,
    ReplyTimeoutError,
    TokensRefundedError,
    ErrorDuringTransaction,
    ErrorInVftGateway,
    ReplyHook,
    GatewayMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
    TransferTokensFailed,
}
