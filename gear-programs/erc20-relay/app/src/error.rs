use super::*;

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Error {
    DecodeReceiptEnvelopeFailure,
    FailedEthTransaction,
    NotSupportedEvent,
    AlreadyProcessed,
    TooOldTransaction,
    SendFailure,
    ReplyFailure,
    HandleResultDecodeFailure,
    MissingCheckpoint,
    InvalidBlockProof,
    TrieDbFailure,
    InvalidReceiptProof,
    ReplyTimeout,
    ReplyHook,
    AbsentVftGateway,
}
