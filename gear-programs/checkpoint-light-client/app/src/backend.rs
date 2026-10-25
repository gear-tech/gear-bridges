use checkpoint_light_client_io::{
    verification::{BackendError, BlsBackend, Request, VerificationError},
    Error,
};
use sails_rs::{gstd::msg, prelude::*};

const BUILTIN_BLS381: ActorId = ActorId::new(hex_literal::hex!(
    "6b6e292c382945e80bf51af2ba7fe9f458dcff81ae6075c46f9095e1bbecdc37"
));

pub(crate) struct GearBls;

impl BlsBackend for GearBls {
    async fn call(&self, request: Request) -> Result<Vec<u8>, BackendError> {
        let request = request.encode();
        msg::send_bytes_for_reply(BUILTIN_BLS381, &request, 0, 0)
            .map_err(|_| BackendError::Unavailable)?
            .await
            .map_err(|_| BackendError::Unavailable)
    }
}

/// Preserve the actor's existing protocol wire errors and builtin-failure trap.
/// A transport or malformed builtin response must never become InvalidSignature.
pub(crate) fn protocol_error(error: VerificationError) -> Error {
    match error {
        VerificationError::Protocol(error) => error,
        VerificationError::Backend(error) => panic!("{error}"),
    }
}
