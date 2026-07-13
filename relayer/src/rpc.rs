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
#[error("{operation} failed with {kind:?} RPC error: {message}")]
pub struct RpcFailure {
    pub operation: &'static str,
    pub kind: RpcFailureKind,
    pub message: String,
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
    if err.chain().any(is_recoverable_error_text) {
        return RetryDecision::Retry;
    }

    if let Some(ethereum_client::Error::ErrorInHTTPTransport(err)) =
        err.downcast_ref::<ethereum_client::Error>()
    {
        return classify_alloy_rpc(err);
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

    RetryDecision::Fail
}

pub fn classify_alloy_rpc(err: &RpcError<TransportErrorKind>) -> RetryDecision {
    match err {
        RpcError::Transport(transport) => match transport {
            TransportErrorKind::MissingBatchResponse(_)
            | TransportErrorKind::BackendGone
            | TransportErrorKind::PubsubUnavailable
            | TransportErrorKind::HttpError(_) => RetryDecision::Retry,
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
        || message.contains("transport error")
        || message.contains("subscription dropped")
        || message.contains("connection refused")
        || message.contains("connection reset")
        || message.contains("broken pipe")
        || message.contains("timed out")
        || message.contains("timeout")
}

pub async fn retry_gear<T, F, Fut>(
    connection: &mut ApiProviderConnection,
    operation: &'static str,
    mut f: F,
) -> anyhow::Result<T>
where
    F: FnMut(GearApi) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0;
    let policy = RetryPolicy::default();

    loop {
        let client = connection.client();
        match f(client).await {
            Ok(value) => return Ok(value),
            Err(err) if classify_anyhow(&err) == RetryDecision::Retry => {
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
                    message: err.to_string(),
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
    fn classifies_real_socket_failures_as_recoverable() {
        let err = anyhow::anyhow!("transport error: connection refused");

        assert_eq!(classify_anyhow(&err), RetryDecision::Retry);
    }
}
