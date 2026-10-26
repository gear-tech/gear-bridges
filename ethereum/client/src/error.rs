use alloy::transports::{RpcError, TransportErrorKind};
use std::{error::Error as StdError, io::ErrorKind};
use thiserror::Error;

/// Retry typed RPC/network failures, never arbitrary I/O or semantic errors.
/// Send failures stay with the original submission's reconciliation owner.
pub fn is_transient_rpc_error(error: &anyhow::Error) -> bool {
    is_transient_rpc_cause(error.as_ref())
}

/// Borrowed counterpart for callers retaining concrete RPC errors.
pub fn is_transient_rpc_cause(error: &(dyn StdError + 'static)) -> bool {
    std::iter::successors(Some(error), |&error| error.source())
        .find_map(|error| {
            if let Some(error) = error.downcast_ref::<Error>() {
                return Some(match error {
                    Error::ErrorInHTTPTransport(error) => transient_alloy_rpc(error),
                    Error::ErrorDuringContractExecution(error)
                    | Error::ErrorQueryingEvent(error)
                    | Error::ChallengeRootPreBroadcast(error) => transient_contract(error),
                    Error::BeaconFinality(error) => is_transient_rpc_error(error),
                    Error::FinalizedEvidenceUnavailable(_) => true,
                    _ => false,
                });
            }
            if let Some(error) = error.downcast_ref::<RpcError<TransportErrorKind>>() {
                return Some(transient_alloy_rpc(error));
            }
            if let Some(error) = error.downcast_ref::<alloy::contract::Error>() {
                return Some(transient_contract(error));
            }
            if error.is::<ethereum_beacon_client::ErrorNotFound>() {
                return Some(true);
            }
            error
                .downcast_ref::<reqwest::Error>()
                .map(|error| is_transient_network_error(error))
                .or_else(|| {
                    error
                        .downcast_ref::<alloy::transports::http::reqwest::Error>()
                        .map(|error| is_transient_network_error(error))
                })
        })
        .unwrap_or(false)
}

fn transient_contract(error: &alloy::contract::Error) -> bool {
    matches!(error, alloy::contract::Error::TransportError(error) if transient_alloy_rpc(error))
}

fn transient_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn transient_alloy_rpc(error: &RpcError<TransportErrorKind>) -> bool {
    match error {
        RpcError::NullResp => true,
        RpcError::Transport(transport) => match transport {
            TransportErrorKind::MissingBatchResponse(_) | TransportErrorKind::BackendGone => true,
            TransportErrorKind::HttpError(error) => transient_status(error.status),
            TransportErrorKind::Custom(error) => is_transient_network_error(error.as_ref()),
            _ => false,
        },
        // JSON-RPC rejections are permanent, whatever their message says.
        _ => false,
    }
}

/// Classify sources only after the caller has established a typed network envelope.
/// Passing an arbitrary storage I/O error here would incorrectly authorize retries.
pub fn is_transient_network_error(error: &(dyn StdError + 'static)) -> bool {
    for error in std::iter::successors(Some(error), |&error| error.source()) {
        if let Some(error) = error.downcast_ref::<RpcError<TransportErrorKind>>() {
            return transient_alloy_rpc(error);
        }
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            if let Some(status) = error.status() {
                return transient_status(status.as_u16());
            }
            if error.is_timeout() || error.is_connect() {
                return true;
            }
            if error.is_builder() || error.is_redirect() || error.is_decode() {
                return false;
            }
        }
        // Alloy uses reqwest 0.12; direct clients still use 0.11. DNS sources
        // need not carry a portable I/O kind, so keep the typed connect envelope.
        if let Some(error) = error.downcast_ref::<alloy::transports::http::reqwest::Error>() {
            if let Some(status) = error.status() {
                return transient_status(status.as_u16());
            }
            if error.is_timeout() || error.is_connect() {
                return true;
            }
            if error.is_builder() || error.is_redirect() || error.is_decode() {
                return false;
            }
        }
        if error.is::<tower::timeout::error::Elapsed>() || error.is::<tokio::time::error::Elapsed>()
        {
            return true;
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            if error.kind() != ErrorKind::Other {
                return matches!(
                    error.kind(),
                    ErrorKind::AddrNotAvailable
                        | ErrorKind::ConnectionRefused
                        | ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::NotConnected
                        | ErrorKind::BrokenPipe
                        | ErrorKind::TimedOut
                        | ErrorKind::UnexpectedEof
                        | ErrorKind::NetworkDown
                        | ErrorKind::NetworkUnreachable
                        | ErrorKind::HostUnreachable
                );
            }
        }
    }
    false
}

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
    #[error("HOLD: Ethereum signer is not configured")]
    SignerNotConfigured,
    #[error("HOLD: Ethereum signing policy admission failed: {0}")]
    SigningPolicy(String),
    #[error("Error during contract execution: {0}")]
    ErrorDuringContractExecution(alloy::contract::Error),
    /// Challenge gas estimation failed before any send was attempted.
    #[error("Challenge failed before broadcast: {0}")]
    ChallengeRootPreBroadcast(alloy::contract::Error),
    #[error("Error sending transaction: {0}")]
    ErrorSendingTransaction(alloy::contract::Error),
    #[error("Error querying event: {0}")]
    ErrorQueryingEvent(alloy::contract::Error),
    #[error("Invalid merkle root observation: {0}")]
    InvalidMerkleRootObservation(String),
    #[error("Error waiting transaction receipt")]
    ErrorWaitingTransactionReceipt,
    #[error("Error fetching transaction")]
    ErrorFetchingTransaction,
    #[error("Error fetching transaction receipt")]
    ErrorFetchingTransactionReceipt,
    #[error("Error fetching block")]
    ErrorFetchingBlock,
    #[error("HOLD: approved Beacon finality is not configured")]
    BeaconFinalityNotConfigured,
    #[error("HOLD: independent Ethereum data witness is not configured")]
    DataWitnessNotConfigured,
    #[error("HOLD: Beacon or authenticated execution evidence failed: {0}")]
    BeaconFinality(#[source] anyhow::Error),
    #[error("HOLD: finalized proof acquisition unavailable: {0}")]
    FinalizedEvidenceUnavailable(&'static str),
    #[error("HOLD: invalid authenticated Ethereum evidence: {0}")]
    InvalidFinalityEvidence(String),
    #[error("Finalized ancestry is not yet verified")]
    FinalizedAncestryPending,
    #[error("Authenticated finalized ancestry conflicts with the original witnessed branch")]
    FinalizedAncestryConflict,
    #[error("Invalid prepared signed transaction: {0}")]
    InvalidPreparedTransaction(String),
    #[error("Failed to build reqwest client")]
    FailedToBuildClient(alloy::transports::http::reqwest::Error),
    #[error("MessageQueue error: {0:x?}")]
    MessageQueue(super::abi::IMessageQueue::IMessageQueueErrors),
}

impl From<RpcError<TransportErrorKind>> for Error {
    fn from(value: RpcError<TransportErrorKind>) -> Self {
        Self::ErrorInHTTPTransport(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_errors_require_a_network_envelope() {
        for kind in [
            ErrorKind::AddrNotAvailable,
            ErrorKind::NetworkDown,
            ErrorKind::NetworkUnreachable,
            ErrorKind::HostUnreachable,
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::NotConnected,
            ErrorKind::BrokenPipe,
            ErrorKind::TimedOut,
            ErrorKind::UnexpectedEof,
        ] {
            assert!(!is_transient_rpc_error(&std::io::Error::from(kind).into()));
            let rpc: RpcError<TransportErrorKind> = RpcError::Transport(
                TransportErrorKind::Custom(Box::new(std::io::Error::from(kind))),
            );
            assert!(is_transient_rpc_error(
                &anyhow::Error::new(rpc).context("read block")
            ));
        }
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::InvalidInput,
            ErrorKind::NotFound,
        ] {
            let rpc: RpcError<TransportErrorKind> = RpcError::Transport(
                TransportErrorKind::Custom(Box::new(std::io::Error::from(kind))),
            );
            assert!(!is_transient_rpc_error(&rpc.into()));
        }
        assert!(!is_transient_rpc_error(&anyhow::anyhow!(
            "transport error: timeout"
        )));
        let rpc = TransportErrorKind::custom_str("network down, connection refused, timeout");
        assert!(!is_transient_rpc_error(&rpc.into()));
    }

    #[test]
    fn permanent_rpc_status_and_send_envelopes_take_precedence() {
        for status in [400, 401, 403, 404, 501] {
            let error = TransportErrorKind::http_error(status, "connection reset, timeout".into());
            assert!(!is_transient_rpc_error(&error.into()));
        }
        for status in [408, 429, 500, 502, 503, 504] {
            let error = TransportErrorKind::http_error(status, String::new());
            assert!(is_transient_rpc_error(&error.into()));
        }
        let rejection: RpcError<TransportErrorKind> = RpcError::ErrorResp(
            serde_json::from_str(r#"{"code":-32603,"message":"network down, timeout"}"#).unwrap(),
        );
        assert!(!is_transient_rpc_error(
            &anyhow::Error::new(rejection).context("read block")
        ));
        let send = Error::ErrorSendingTransaction(alloy::contract::Error::TransportError(
            TransportErrorKind::custom(std::io::Error::from(ErrorKind::AddrNotAvailable)),
        ));
        assert!(!is_transient_rpc_error(&send.into()));
        assert!(!is_transient_rpc_error(
            &Error::FinalizedAncestryConflict.into()
        ));
    }

    #[test]
    fn challenge_prebroadcast_transience_does_not_authorize_send_retries() {
        let transport = || {
            alloy::contract::Error::TransportError(TransportErrorKind::http_error(
                503,
                "temporarily unavailable".into(),
            ))
        };
        assert!(is_transient_rpc_cause(&Error::ChallengeRootPreBroadcast(
            transport()
        )));
        assert!(!is_transient_rpc_cause(&Error::ErrorSendingTransaction(
            transport()
        )));
        let rejection = alloy::contract::Error::TransportError(RpcError::ErrorResp(
            serde_json::from_str(r#"{"code":-32603,"message":"timeout"}"#).unwrap(),
        ));
        assert!(!is_transient_rpc_cause(&Error::ChallengeRootPreBroadcast(
            rejection
        )));
    }

    #[tokio::test]
    async fn timeout_requires_a_network_envelope() {
        let elapsed = tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
            .await
            .unwrap_err();
        let deadline = anyhow::Error::new(elapsed).context("startup authentication deadline");
        assert!(!is_transient_rpc_error(&deadline));
        let rpc =
            TransportErrorKind::custom(deadline.downcast::<tokio::time::error::Elapsed>().unwrap());
        assert!(is_transient_rpc_error(&rpc.into()));
    }

    #[tokio::test]
    async fn actual_reqwest_unavailable_local_address_is_transient() {
        use std::net::{IpAddr, Ipv4Addr};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        // Bind an unassigned documentation address, not the machine's changing Wi-Fi interface.
        let error = reqwest::Client::builder()
            .no_proxy()
            .local_address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!("http://{}", listener.local_addr().unwrap()))
            .send()
            .await
            .unwrap_err();
        assert!(is_transient_rpc_cause(&error));
        let rpc = RpcError::Transport(TransportErrorKind::Custom(Box::new(error)));
        let error = Error::ErrorQueryingEvent(alloy::contract::Error::TransportError(rpc));
        assert!(is_transient_rpc_error(
            &anyhow::Error::new(error).context("deposit read")
        ));
    }

    #[tokio::test]
    async fn actual_alloy_reqwest_dns_failure_reconnects_without_retrying_storage_errors() {
        use alloy::transports::http::reqwest::{
            dns::{Name, Resolve, Resolving},
            Client,
        };
        struct UnavailableDns;
        impl Resolve for UnavailableDns {
            fn resolve(&self, _: Name) -> Resolving {
                Box::pin(async { Err(std::io::Error::other("resolver unavailable").into()) })
            }
        }
        let error = Client::builder()
            .no_proxy()
            .dns_resolver(std::sync::Arc::new(UnavailableDns))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
            .get("https://bridge-dns.invalid")
            .send()
            .await
            .unwrap_err();
        assert!(error.is_connect(), "{error:?}");
        assert!(is_transient_rpc_cause(&error));
        let rpc = TransportErrorKind::custom(error);
        assert!(is_transient_rpc_error(&rpc.into()));
        assert!(!is_transient_rpc_error(
            &std::io::Error::other("resolver unavailable").into()
        ));
    }
}
