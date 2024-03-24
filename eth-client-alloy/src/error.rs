#[derive(Debug)]
pub enum VerifierError {
    ErrorInHTTPTransport,
    WrongAddress,
    WrongPrivateKey,
    ErrorDuringContractExecution,
    ErrorSendingTransaction,
    ErrorWaitingTransactionReceipt,
    WrongPathToFile,
    WrongJsonFormation,
    UnableToConvertToU256,
}
