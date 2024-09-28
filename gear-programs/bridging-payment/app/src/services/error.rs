use sails_rs::prelude::*;
use vft_gateway_client::Error as VftGatewayError;

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
