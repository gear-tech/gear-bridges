use super::vft_gateway::Error as VftGatewayError;
use sails_rs::prelude::*;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToGateWayDecode,
    ReplyTimeout,
    Gateway(VftGatewayError),
}

impl From<VftGatewayError> for Error {
    fn from(e: VftGatewayError) -> Self {
        Self::Gateway(e)
    }
}
