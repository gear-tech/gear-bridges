use alloy::transports::{RpcError, TransportErrorKind};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error in HTTP transport: {0}")]
    ErrorInHTTPTransport(RpcError<TransportErrorKind>),
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
    #[error("Error querying event: {0}")]
    ErrorQueryingEvent(alloy::contract::Error),
    #[error("Error waiting transaction receipt")]
    ErrorWaitingTransactionReceipt,
    #[error("Error fetching transaction")]
    ErrorFetchingTransaction,
    #[error("Error fetching transaction receipt")]
    ErrorFetchingTransactionReceipt,
    #[error("Error fetching block")]
    ErrorFetchingBlock,
    #[error("Failed to build reqwest client")]
    FailedToBuildClient(alloy::transports::http::reqwest::Error),
}

impl From<RpcError<TransportErrorKind>> for Error {
    fn from(value: RpcError<TransportErrorKind>) -> Self {
        Self::ErrorInHTTPTransport(value)
    }
}
