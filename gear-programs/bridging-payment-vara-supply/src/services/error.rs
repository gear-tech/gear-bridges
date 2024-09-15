use sails_rs::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToTreasuryDecode,
    ReplyTimeout,
    ReplyHook,
    TreasuryMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
}
