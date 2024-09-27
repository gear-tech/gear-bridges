use sails_rs::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToGateWayDecode,
    ReplyTimeout,
    ReplyHook,
    GatewayMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
}
