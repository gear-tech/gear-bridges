use sails_rs::prelude::*;
use vft_manager_client::Error as VftManagerError;

/// Error types for Bridging Payment service.
#[derive(Debug, Encode, Decode, TypeInfo)]
pub enum Error {
    /// Failed to send message to a program.
    SendFailure,
    /// Error while waiting for reply from the program.
    ReplyFailure,
    /// Failed to set reply timeout.
    ReplyTimeout,
    /// Failed to decode reply from `vft-manager`.
    RequestToVftManagerDecode,
    /// `vft-manager` replied error.
    VftManager(VftManagerError),
}

impl From<VftManagerError> for Error {
    fn from(e: VftManagerError) -> Self {
        Self::VftManager(e)
    }
}
