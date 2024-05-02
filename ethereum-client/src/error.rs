#[derive(Debug)]
pub enum Error {
    ErrorInHTTPTransport,
    WrongAddress,
    WrongNodeUrl,
    WrongPrivateKey,
    ErrorDuringContractExecution(alloy_contract::Error),
    ErrorSendingTransaction,
    ErrorWaitingTransactionReceipt,
    WrongPathToFile,
    WrongJsonFormation,
    UnableToConvertToU256,
}
