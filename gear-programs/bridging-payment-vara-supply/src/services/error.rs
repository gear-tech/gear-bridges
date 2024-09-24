use sails_rs::prelude::*;

use super::vft_treasury::Error as VftTreasuryError;

//#[allow(clippy::enum_variant_names)]
#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    SendFailure,
    ReplyFailure,
    RequestToTreasuryDecode,
    ReplyTimeout,
    ReplyHook,
    TreasuryMessageProcessingFailed,
    InvalidMessageStatus,
    MessageNotFound,
    Treasury(VftTreasuryError),
}

impl From<VftTreasuryError> for Error {
    fn from(e: VftTreasuryError) -> Self {
        Self::Treasury(e)
    }
}
