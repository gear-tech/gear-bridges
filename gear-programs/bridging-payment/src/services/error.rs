use sails_rs::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    TransferTokensDecode,
    TokensTransferFailure,
    RequestToGateWayDecode,
    PayloadSize,
    MintTokensDecode,
    ReplyTimeout,
    TokensRefunded,
    TransactionFailure,
    FailureInVftGateway,
    ReplyHook,
    GatewayMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
    TransferTokensFailed,
}
