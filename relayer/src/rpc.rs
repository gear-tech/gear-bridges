use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy::transports::{RpcError, TransportErrorKind};
use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::GearApi;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcFailureKind {
    Recoverable,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Fail,
}

#[derive(Debug, Error)]
#[error("{operation} failed with {kind:?} operation error: {source}")]
pub struct RpcFailure {
    pub operation: &'static str,
    pub kind: RpcFailureKind,
    #[source]
    pub source: anyhow::Error,
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    fn delay(&self, attempt: u32) -> Duration {
        let multiplier = 1u32.checked_shl(attempt.min(6)).unwrap_or(64);
        let delay = self
            .base_delay
            .saturating_mul(multiplier)
            .min(self.max_delay);
        let jitter_bound = delay.as_millis().min(1_000) as u64 + 1;
        let jitter_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos() as u64 % jitter_bound)
            .unwrap_or_default();
        delay + Duration::from_millis(jitter_ms)
    }
}

pub fn classify_anyhow(err: &anyhow::Error) -> RetryDecision {
    if let Some(err) = err.downcast_ref::<RpcFailure>() {
        return match err.kind {
            RpcFailureKind::Recoverable => RetryDecision::Retry,
            RpcFailureKind::Permanent => RetryDecision::Fail,
        };
    }

    if let Some(err) = err.downcast_ref::<ethereum_client::Error>() {
        return classify_ethereum_error(err);
    }

    if let Some(err) = err.downcast_ref::<RpcError<TransportErrorKind>>() {
        return classify_alloy_rpc(err);
    }

    if let Some(gclient::Error::GearSDK(gsdk::Error::Subxt(err))) =
        err.downcast_ref::<gclient::Error>()
    {
        if is_recoverable_subxt(err) {
            return RetryDecision::Retry;
        }
    }

    if let Some(gsdk::Error::Subxt(err)) = err.downcast_ref::<gsdk::Error>() {
        if is_recoverable_subxt(err) {
            return RetryDecision::Retry;
        }
    }

    if let Some(err) = err.downcast_ref::<subxt::Error>() {
        if is_recoverable_subxt(err) {
            return RetryDecision::Retry;
        }
    }

    if err.chain().any(is_recoverable_error_text) {
        return RetryDecision::Retry;
    }

    RetryDecision::Fail
}

pub fn classify_ethereum_error(err: &ethereum_client::Error) -> RetryDecision {
    match err {
        ethereum_client::Error::ErrorInHTTPTransport(err) => classify_alloy_rpc(err),
        ethereum_client::Error::ErrorDuringContractExecution(err)
        | ethereum_client::Error::ErrorSendingTransaction(err)
        | ethereum_client::Error::ErrorQueryingEvent(err) => match err {
            alloy::contract::Error::TransportError(err) => classify_alloy_rpc(err),
            _ => RetryDecision::Fail,
        },
        _ => RetryDecision::Fail,
    }
}

pub fn classify_alloy_rpc(err: &RpcError<TransportErrorKind>) -> RetryDecision {
    match err {
        RpcError::Transport(transport) => match transport {
            TransportErrorKind::MissingBatchResponse(_) | TransportErrorKind::BackendGone => {
                RetryDecision::Retry
            }
            TransportErrorKind::HttpError(err)
                if err.status == 429 || matches!(err.status, 500 | 502 | 503 | 504) =>
            {
                RetryDecision::Retry
            }
            TransportErrorKind::Custom(message) if is_recoverable_error_text(message) => {
                RetryDecision::Retry
            }
            _ => RetryDecision::Fail,
        },
        RpcError::ErrorResp(_) => RetryDecision::Fail,
        RpcError::NullResp => RetryDecision::Retry,
        _ => RetryDecision::Fail,
    }
}

pub fn is_recoverable_error_text(message: impl std::fmt::Display) -> bool {
    let message = message.to_string().to_ascii_lowercase();
    message.contains("background task closed")
        || message.contains("connection closed")
        || message.contains("disconnectedwillreconnect")
        || message.contains("disconnected will reconnect")
        || message.contains("restart required")
        || message.contains("backend gone")
        || message.contains("backend connection task has stopped")
        || message.contains("transport error")
        || message.contains("subscription dropped")
        || message.contains("connection refused")
        || message.contains("connection reset")
        || message.contains("broken pipe")
        || message.contains("timed out")
        || message.contains("timeout")
}

/// Retry an Ethereum operation without dropping its in-memory work item.
///
/// This helper intentionally waits until a recoverable outage ends. Callers in a
/// multiplexed event loop should instead bound reconnection and reschedule the work.
pub async fn retry_eth<T, F, Fut>(
    api: &mut ethereum_client::EthApi,
    operation: &'static str,
    mut f: F,
) -> Result<T, ethereum_client::Error>
where
    F: FnMut(ethereum_client::EthApi) -> Fut,
    Fut: Future<Output = Result<T, ethereum_client::Error>>,
{
    let mut attempt = 0;
    let policy = RetryPolicy::default();

    loop {
        match f(api.clone()).await {
            Ok(value) => return Ok(value),
            Err(err) if classify_ethereum_error(&err) == RetryDecision::Retry => {
                let delay = policy.delay(attempt);
                log::warn!(
                    "{operation} failed with recoverable Ethereum RPC error: {err}. Reconnecting in {delay:?}"
                );
                tokio::time::sleep(delay).await;

                loop {
                    match api.reconnect().await {
                        Ok(reconnected) => {
                            *api = reconnected;
                            attempt = attempt.saturating_add(1);
                            log::info!(
                                "{operation}: reconnected to Ethereum API, retrying operation"
                            );
                            break;
                        }
                        Err(err) if classify_ethereum_error(&err) == RetryDecision::Retry => {
                            let delay = policy.delay(attempt);
                            log::warn!(
                                "{operation} failed to reconnect to Ethereum after a recoverable RPC error: {err}. Retrying in {delay:?}"
                            );
                            tokio::time::sleep(delay).await;
                            attempt = attempt.saturating_add(1);
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
            Err(err) => return Err(err),
        }
    }
}

pub async fn retry_gear<T, F, Fut>(
    connection: &mut ApiProviderConnection,
    operation: &'static str,
    f: F,
) -> anyhow::Result<T>
where
    F: FnMut(GearApi) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    retry_gear_if(connection, operation, f, |err| {
        classify_anyhow(err) == RetryDecision::Retry
    })
    .await
}

pub async fn retry_gear_if<T, F, Fut, P>(
    connection: &mut ApiProviderConnection,
    operation: &'static str,
    mut f: F,
    should_retry: P,
) -> anyhow::Result<T>
where
    F: FnMut(GearApi) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
    P: Fn(&anyhow::Error) -> bool,
{
    let mut attempt = 0;
    let policy = RetryPolicy::default();

    loop {
        let client = connection.client();
        match f(client).await {
            Ok(value) => return Ok(value),
            Err(err) if should_retry(&err) => {
                let delay = policy.delay(attempt);
                log::warn!(
                    "{operation} failed with recoverable Gear RPC error: {err}. Reconnecting in {delay:?}"
                );
                tokio::time::sleep(delay).await;
                connection.reconnect().await.map_err(|reconnect_err| {
                    anyhow::anyhow!(
                        "{} failed to reconnect after recoverable RPC error: {reconnect_err}",
                        operation
                    )
                })?;
                attempt = attempt.saturating_add(1);
            }
            Err(err) => {
                return Err(RpcFailure {
                    operation,
                    kind: RpcFailureKind::Permanent,
                    source: err,
                }
                .into());
            }
        }
    }
}

fn is_recoverable_subxt(err: &subxt::Error) -> bool {
    if err.is_disconnected_will_reconnect() {
        return true;
    }

    matches!(
        err,
        subxt::Error::Rpc(subxt::error::RpcError::SubscriptionDropped)
            | subxt::Error::Rpc(subxt::error::RpcError::ClientError(
                gsdk::ext::subxt_rpcs::Error::DisconnectedWillReconnect(_)
            ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_disconnect_text_as_recoverable() {
        let err = anyhow::anyhow!(
            "RPC error: client error: The background task closed connection closed; restart required"
        );

        assert_eq!(classify_anyhow(&err), RetryDecision::Retry);
    }

    #[test]
    fn does_not_treat_plain_channel_closure_as_rpc_recoverable() {
        let err = anyhow::anyhow!("No active receivers for Gear block listener");

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
    }

    #[test]
    fn classifies_backend_gone_as_recoverable() {
        let err = RpcError::Transport(TransportErrorKind::BackendGone);

        assert_eq!(classify_alloy_rpc(&err), RetryDecision::Retry);
    }

    #[test]
    fn classifies_only_retryable_http_statuses_as_recoverable() {
        for status in [429, 500, 502, 503, 504] {
            let err = TransportErrorKind::http_error(status, String::new());
            assert_eq!(classify_alloy_rpc(&err), RetryDecision::Retry);
        }

        for status in [400, 401, 403, 404, 501] {
            let err = TransportErrorKind::http_error(status, String::new());
            assert_eq!(classify_alloy_rpc(&err), RetryDecision::Fail);
        }
    }

    #[test]
    fn treats_missing_pubsub_support_as_permanent() {
        let err = TransportErrorKind::pubsub_unavailable();

        assert_eq!(classify_alloy_rpc(&err), RetryDecision::Fail);
    }

    #[test]
    fn keeps_permanent_rpc_failure_permanent_even_when_message_says_timeout() {
        let err = anyhow::Error::new(RpcFailure {
            operation: "proof storage",
            kind: RpcFailureKind::Permanent,
            source: anyhow::anyhow!("write timed out"),
        });

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
    }

    #[test]
    fn explicit_rpc_failure_kind_overrides_recoverable_source() {
        let source = ethereum_client::Error::ErrorInHTTPTransport(RpcError::Transport(
            TransportErrorKind::BackendGone,
        ));
        let permanent = anyhow::Error::new(RpcFailure {
            operation: "proof storage",
            kind: RpcFailureKind::Permanent,
            source: anyhow::Error::new(source),
        });

        assert_eq!(classify_anyhow(&permanent), RetryDecision::Fail);

        let recoverable = anyhow::Error::new(RpcFailure {
            operation: "RPC request",
            kind: RpcFailureKind::Recoverable,
            source: anyhow::anyhow!("non-recoverable source text"),
        });
        assert_eq!(classify_anyhow(&recoverable), RetryDecision::Retry);
    }

    #[test]
    fn classifies_nested_contract_backend_gone_as_recoverable() {
        let err = ethereum_client::Error::ErrorDuringContractExecution(
            alloy::contract::Error::TransportError(RpcError::Transport(
                TransportErrorKind::BackendGone,
            )),
        );

        assert_eq!(classify_ethereum_error(&err), RetryDecision::Retry);
        assert_eq!(classify_anyhow(&err.into()), RetryDecision::Retry);
    }

    #[test]
    fn keeps_typed_error_response_permanent_even_when_message_says_timeout() {
        let err = ethereum_client::Error::ErrorDuringContractExecution(
            alloy::contract::Error::TransportError(RpcError::ErrorResp(
                serde_json::from_str(r#"{"code":-32603,"message":"timeout"}"#).unwrap(),
            )),
        );

        assert_eq!(classify_ethereum_error(&err), RetryDecision::Fail);
        assert_eq!(classify_anyhow(&err.into()), RetryDecision::Fail);
    }

    #[test]
    fn classifies_backend_task_stopped_text_as_recoverable() {
        let err = anyhow::anyhow!("backend connection task has stopped");

        assert_eq!(classify_anyhow(&err), RetryDecision::Retry);
    }

    #[test]
    fn classifies_real_socket_failures_as_recoverable() {
        let err = anyhow::anyhow!("transport error: connection refused");

        assert_eq!(classify_anyhow(&err), RetryDecision::Retry);
    }
}
