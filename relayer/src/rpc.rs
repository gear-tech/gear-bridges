use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy::transports::{RpcError, TransportErrorKind};
use ethereum_client::EthApi;
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

    if err.chain().any(is_permanent_error_text) {
        return RetryDecision::Fail;
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

    // Default to retry for unknown errors — transient RPC failures are more common
    // than permanent ones, and retrying a permanent error is harmless (it just fails again).
    RetryDecision::Retry
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
        || message.contains("internal error")
        || message.contains("state not available")
        || message.contains("block not found")
        || message.contains("not available")
}

pub fn is_permanent_error_text(message: impl std::fmt::Display) -> bool {
    let message = message.to_string().to_ascii_lowercase();
    message.contains("invalid params")
        || message.contains("method not found")
        || message.contains("parse error")
        || message.contains("no active receivers")
        || message.contains("channel closed")
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

/// Maximum Ethereum reconnect attempts before giving up.
pub const ETH_RECONNECT_RETRIES: u32 = 5;

/// Reconnect `eth_api`, retrying with backoff up to `ETH_RECONNECT_RETRIES` times.
/// Returns Ok(()) on success; Err after all retries exhausted.
pub async fn reconnect_eth(eth_api: &mut EthApi, who: &str) -> anyhow::Result<()> {
    let policy = RetryPolicy::default();
    for attempt in 0..ETH_RECONNECT_RETRIES {
        match eth_api.reconnect().await {
            Ok(new) => {
                *eth_api = new;
                log::info!(
                    "{who}: reconnected to Ethereum (attempt {})",
                    attempt + 1
                );
                return Ok(());
            }
            Err(e) => {
                let delay = policy.delay(attempt);
                let n = attempt + 1;
                log::warn!(
                    "{who}: Ethereum reconnect failed: {e}; retrying in {delay:?} ({n}/{ETH_RECONNECT_RETRIES})"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
    anyhow::bail!("{who}: Ethereum reconnect failed after {ETH_RECONNECT_RETRIES} attempts")
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

    #[test]
    fn classifies_context_wrapped_transient_error_as_recoverable() {
        // Simulates gear_rpc_client::GearApi::authority_set_id which wraps errors with
        // .context("Failed to fetch authority set id"). The underlying RPC error is
        // transient but doesn't match any known recoverable pattern.
        let inner = anyhow::anyhow!("internal error: state missing for block 0xabc");
        let err = inner.context("Failed to fetch authority set id");

        assert_eq!(classify_anyhow(&err), RetryDecision::Retry);
    }

    #[test]
    fn does_not_retry_known_permanent_errors() {
        let err = anyhow::anyhow!("Invalid params: method not found");

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
    }
}
