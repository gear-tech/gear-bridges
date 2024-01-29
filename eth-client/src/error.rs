#[derive(Debug)]
pub enum VerifierError {
    ErrorInHTTPTransport,
    WrongAddress,
    ErrorDuringContractExecution,
    WrongPathToFile,
    WrongJsonFormation,
    UnableToConvertToU256,
}
