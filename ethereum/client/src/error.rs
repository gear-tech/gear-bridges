use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Not implemented")]
    NotImplemented,
    #[error("Error in HTTP transport")]
    ErrorInHTTPTransport,
    #[error("Wrong address")]
    WrongAddress,
    #[error("Wrong node URL")]
    WrongNodeUrl,
    #[error("Wrong private key")]
    WrongPrivateKey,
    #[error("Error during contract execution: {0}")]
    ErrorDuringContractExecution(alloy::contract::Error),
    #[error("Error sending transaction: {0}")]
    ErrorSendingTransaction(alloy::contract::Error),
    #[error("Error waiting transaction receipt")]
    ErrorWaitingTransactionReceipt,
    #[error("Error fetching transaction")]
    ErrorFetchingTransaction,
    #[error("Error fetching transaction receipt")]
    ErrorFetchingTransactionReceipt,
    #[error("Error fetching block")]
    ErrorFetchingBlock,
    #[error("Wrong path to file")]
    WrongPathToFile,
    #[error("Wrong JSON format")]
    WrongJsonFormation,
    #[error("Cannot convert to U256")]
    UnableToConvertToU256,
}
