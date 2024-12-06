use super::{Decode, Encode, TypeInfo};

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Error {
    DecodeReceiptEnvelopeFailure,
    FailedEthTransaction,
    AlreadyProcessed,
    SendFailure,
    ReplyFailure,
    HandleResultDecodeFailure,
    MissingCheckpoint,
    InvalidBlockProof,
    TrieDbFailure,
    InvalidReceiptProof,
    ReplyTimeout,
    ReplyHook,
    InvalidMessage,
}
