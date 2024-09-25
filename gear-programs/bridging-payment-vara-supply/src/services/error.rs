use sails_rs::prelude::*;

use super::vft_treasury::Error as VftTreasuryError;

#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToTreasuryDecode,
    ReplyTimeout,
    Treasury(VftTreasuryError),
}

impl From<VftTreasuryError> for Error {
    fn from(e: VftTreasuryError) -> Self {
        Self::Treasury(e)
    }
}
