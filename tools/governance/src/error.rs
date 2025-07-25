use alloy::{
    contract::Error as AlloyContractError,
    transports::{RpcError as AlloyRpcError, TransportErrorKind},
};
use std::io::Error as IOError;
use toml::de::Error as TomlError;

/// Custom Result type with two generic parameters for user convenience
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Describes possible errors that might happen when user interacts with this crate
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    IO(#[from] IOError),
    #[error("Toml deserialization error: {0}")]
    Toml(#[from] TomlError),
    #[error("Ethereum RPC error: {0}")]
    Rpc(#[from] AlloyRpcError<TransportErrorKind>),
    #[error("Ethereum contract error: {0}")]
    Contract(#[from] AlloyContractError),
}
