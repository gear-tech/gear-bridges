use sails_rs::prelude::*;
use vft_manager_client::Error as VftManagerError;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToVftManagerDecode,
    ReplyTimeout,
    VftManager(VftManagerError),
}

impl From<VftManagerError> for Error {
    fn from(e: VftManagerError) -> Self {
        Self::VftManager(e)
    }
}
