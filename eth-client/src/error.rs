#[derive(Debug)]
pub enum VerifierError {
    ErrorInHTTPTransport,
    WrongAddress,
    ErrorDuringContractExecution,
}
