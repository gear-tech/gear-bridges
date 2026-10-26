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
        (delay + Duration::from_millis(jitter_ms)).min(self.max_delay)
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

    if classify_gear_transport_error(err) == RetryDecision::Retry {
        return RetryDecision::Retry;
    }

    if ethereum_client::is_transient_rpc_error(err) {
        return RetryDecision::Retry;
    }

    RetryDecision::Fail
}

pub(crate) fn classify_gear_transport_error(err: &anyhow::Error) -> RetryDecision {
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

    if let Some(err) = err.downcast_ref::<gsdk::ext::subxt_rpcs::Error>() {
        if is_recoverable_gear_rpc(err) {
            return RetryDecision::Retry;
        }
    }

    RetryDecision::Fail
}

pub fn classify_ethereum_error(err: &ethereum_client::Error) -> RetryDecision {
    if ethereum_client::error::is_transient_rpc_cause(err) {
        RetryDecision::Retry
    } else {
        RetryDecision::Fail
    }
}

pub fn classify_alloy_rpc(err: &RpcError<TransportErrorKind>) -> RetryDecision {
    if ethereum_client::error::is_transient_rpc_cause(err) {
        RetryDecision::Retry
    } else {
        RetryDecision::Fail
    }
}

/// Retry an Ethereum operation without dropping its in-memory work item.
///
/// This helper intentionally waits until a recoverable outage ends.
pub async fn retry_eth<T, F, Fut>(
    api: &mut ethereum_client::EthApi,
    operation: &'static str,
    f: F,
) -> Result<T, ethereum_client::Error>
where
    F: FnMut(ethereum_client::EthApi) -> Fut,
    Fut: Future<Output = Result<T, ethereum_client::Error>>,
{
    retry(
        api,
        operation,
        RetryPolicy::default(),
        None,
        f,
        |api| async move { api.reconnect().await },
        classify_ethereum_error,
    )
    .await
}
/// Retry a read without unwinding the listener's cursor or queued work.
pub async fn retry_polling_eth<T, F, Fut>(
    api: &mut ethereum_client::PollingEthApi,
    operation: &'static str,
    f: F,
) -> anyhow::Result<T>
where
    F: FnMut(ethereum_client::PollingEthApi) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    retry(
        api,
        operation,
        RetryPolicy::default(),
        None,
        f,
        |api| async move { api.reconnect().await },
        classify_anyhow,
    )
    .await
}

/// Reconnect a polling Ethereum client and retain the replacement provider.
pub async fn reconnect_polling_eth(api: &mut ethereum_client::PollingEthApi) -> anyhow::Result<()> {
    *api = api.reconnect().await?;
    Ok(())
}

async fn retry<T, A, E, F, Fut, R, ReconnectFut, C>(
    api: &mut A,
    operation: &'static str,
    policy: RetryPolicy,
    max_retries: Option<u32>,
    mut f: F,
    mut reconnect: R,
    classify: C,
) -> Result<T, E>
where
    A: Clone,
    E: std::fmt::Display,
    F: FnMut(A) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    R: FnMut(A) -> ReconnectFut,
    ReconnectFut: Future<Output = Result<A, E>>,
    C: Fn(&E) -> RetryDecision,
{
    let mut attempts = 0;

    loop {
        match f(api.clone()).await {
            Ok(value) => return Ok(value),
            Err(err) if classify(&err) == RetryDecision::Retry => {
                if max_retries.is_some_and(|max| attempts >= max) {
                    return Err(err);
                }

                let delay = policy.delay(attempts);
                attempts = attempts.saturating_add(1);
                log::warn!(
                    "{operation} failed with recoverable RPC error: {err}. Reconnecting in {delay:?}"
                );
                tokio::time::sleep(delay).await;

                loop {
                    match reconnect(api.clone()).await {
                        Ok(reconnected) => {
                            *api = reconnected;
                            log::info!("{operation}: reconnected, retrying operation");
                            break;
                        }
                        Err(err) if classify(&err) == RetryDecision::Retry => {
                            if max_retries.is_some_and(|max| attempts >= max) {
                                return Err(err);
                            }

                            let delay = policy.delay(attempts);
                            attempts = attempts.saturating_add(1);
                            log::warn!(
                                "{operation} failed to reconnect after a recoverable RPC error: {err}. Retrying in {delay:?}"
                            );
                            tokio::time::sleep(delay).await;
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

    match err {
        subxt::Error::Rpc(subxt::error::RpcError::SubscriptionDropped) => true,
        subxt::Error::Rpc(subxt::error::RpcError::ClientError(err)) => is_recoverable_gear_rpc(err),
        _ => false,
    }
}

fn is_recoverable_gear_rpc(err: &gsdk::ext::subxt_rpcs::Error) -> bool {
    use gsdk::ext::{jsonrpsee::core::client::Error as ClientError, subxt_rpcs::Error};
    match err {
        Error::DisconnectedWillReconnect(_) => true,
        Error::Client(err) => matches!(
            err.downcast_ref::<ClientError>(),
            Some(
                ClientError::Transport(_)
                    | ClientError::RestartNeeded(_)
                    | ClientError::RequestTimeout
            )
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_raw_and_wrapped_gear_disconnects_but_not_semantic_errors() {
        use gsdk::ext::{jsonrpsee::core::client::Error as ClientError, subxt_rpcs::Error};
        let recoverable: [fn() -> Error; 4] = [
            || Error::DisconnectedWillReconnect("disconnected".into()),
            || {
                Error::Client(Box::new(ClientError::Transport(Box::new(
                    std::io::Error::from(std::io::ErrorKind::ConnectionReset),
                ))))
            },
            || {
                Error::Client(Box::new(ClientError::RestartNeeded(std::sync::Arc::new(
                    ClientError::RequestTimeout,
                ))))
            },
            || Error::Client(Box::new(ClientError::RequestTimeout)),
        ];
        let permanent: [fn() -> Error; 2] = [
            || Error::Client(Box::new(ClientError::Custom("restart required".into()))),
            || {
                Error::User(
                    serde_json::from_str(r#"{"code":-32000,"message":"request timed out"}"#)
                        .unwrap(),
                )
            },
        ];
        for (factories, expected) in [
            (recoverable.as_slice(), RetryDecision::Retry),
            (permanent.as_slice(), RetryDecision::Fail),
        ] {
            for make_error in factories {
                for wrapper in 0..4 {
                    let raw = make_error();
                    let error: anyhow::Error = if wrapper == 0 {
                        raw.into()
                    } else {
                        let subxt = subxt::Error::Rpc(subxt::error::RpcError::ClientError(raw));
                        match wrapper {
                            1 => subxt.into(),
                            2 => gsdk::Error::Subxt(Box::new(subxt)).into(),
                            _ => {
                                gclient::Error::GearSDK(gsdk::Error::Subxt(Box::new(subxt))).into()
                            }
                        }
                    };
                    assert_eq!(classify_anyhow(&error.context("source query")), expected);
                }
            }
        }
    }

    #[test]
    fn does_not_classify_untyped_disconnect_text_as_rpc() {
        let err = anyhow::anyhow!(
            "RPC error: client error: The background task closed connection closed; restart required"
        );

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
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
        for status in [408, 429, 500, 502, 503, 504] {
            let err = TransportErrorKind::http_error(status, String::new());
            assert_eq!(classify_alloy_rpc(&err), RetryDecision::Retry);
        }

        for status in [400, 401, 403, 404, 501] {
            let err = TransportErrorKind::http_error(status, String::new());
            assert_eq!(classify_alloy_rpc(&err), RetryDecision::Fail);
        }
    }

    fn identity_rpc_app(genesis: alloy::rpc::types::Block) -> axum::Router {
        axum::Router::new().route(
            "/",
            axum::routing::post(move |axum::Json(request): axum::Json<serde_json::Value>| {
                let genesis = genesis.clone();
                async move {
                    let result = match request["method"].as_str().unwrap() {
                        "eth_chainId" => serde_json::json!("0x7a69"),
                        "eth_getBlockByNumber" => {
                            assert!(matches!(
                                request["params"][0].as_str(),
                                Some("0x0" | "finalized" | "latest")
                            ));
                            serde_json::json!(genesis)
                        }
                        "eth_getLogs" => serde_json::json!([]),
                        method => panic!("unexpected identity fixture RPC: {method}"),
                    };
                    axum::Json(serde_json::json!({
                        "jsonrpc": "2.0", "id": request["id"], "result": result
                    }))
                }
            }),
        )
    }

    #[tokio::test]
    async fn classifies_http_errors_from_actual_event_readers() {
        for (status, expected) in [(429, RetryDecision::Retry), (401, RetryDecision::Fail)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let app = axum::Router::new().route(
                "/",
                axum::routing::post(move || async move {
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        "RPC unavailable",
                    )
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut genesis: alloy::rpc::types::Block = Default::default();
            genesis.header.inner.extra_data = uuid::Uuid::new_v4().as_bytes().to_vec().into();
            genesis.header.hash = genesis.header.inner.hash_slow();
            let genesis_hash = genesis.header.hash;
            let witness_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let witness_endpoint = format!("http://{}", witness_listener.local_addr().unwrap());
            let witness = tokio::spawn(async move {
                axum::serve(witness_listener, identity_rpc_app(genesis))
                    .await
                    .unwrap()
            });
            let api = ethereum_client::PollingEthApi::new_with_retries(&endpoint, None, None)
                .await
                .unwrap()
                .with_data_witness(&witness_endpoint, 31337, genesis_hash)
                .await
                .unwrap();
            let paid = api
                .fetch_fee_paid_events_txs_at(Default::default(), 1, Default::default())
                .await
                .unwrap_err();
            let deposit = api
                .fetch_deposit_events_at(Default::default(), 1, Default::default())
                .await
                .unwrap_err();
            let signing_client = ethereum_client::EthApi::new(
                &endpoint,
                "0x0000000000000000000000000000000000000001",
                None,
                None,
                None,
            )
            .await
            .err()
            .expect("HTTP status must reject signing-client initialization");
            assert_eq!(
                classify_anyhow(&anyhow::Error::new(signing_client)),
                expected
            );
            server.abort();
            witness.abort();
            assert_eq!(classify_anyhow(&paid), expected);
            assert_eq!(classify_anyhow(&deposit), expected);
        }
    }

    #[tokio::test]
    async fn retries_nested_socket_resets_from_actual_http_readers() {
        use tokio::io::AsyncReadExt;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let bootstrap_listener =
            tokio::net::TcpListener::from_std(listener.try_clone().unwrap()).unwrap();
        let mut header: alloy::rpc::types::Header = Default::default();
        header.inner.extra_data = uuid::Uuid::new_v4().as_bytes().to_vec().into();
        header.hash = header.inner.hash_slow();
        let genesis: alloy::rpc::types::Block = alloy::rpc::types::Block {
            header: header.clone(),
            ..Default::default()
        };
        let app = identity_rpc_app(genesis);
        let witness_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let witness_endpoint = format!("http://{}", witness_listener.local_addr().unwrap());
        let witness_app = app.clone();
        let witness =
            tokio::spawn(async move { axum::serve(witness_listener, witness_app).await.unwrap() });
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let bootstrap = tokio::spawn(async move {
            axum::serve(bootstrap_listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let genesis_hash = header.hash;
        let beacon = ethereum_client::beacon_finality::test_fixtures::LocalBeaconFixture::new(
            31337,
            header.hash,
            std::sync::Arc::new(move || header.clone()),
        )
        .await
        .unwrap();
        let api = ethereum_client::PollingEthApi::new(&endpoint)
            .await
            .unwrap()
            .with_beacon_finality(beacon.config.clone(), &beacon.directory)
            .await
            .unwrap()
            .with_data_witness(&witness_endpoint, 31337, genesis_hash)
            .await
            .unwrap();
        stop.send(()).unwrap();
        bootstrap.await.unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                // Leave the rest of the HTTP request unread so closing resets the connection.
                let _ = socket.read_exact(&mut [0; 1]).await;
            }
        });
        let block = api.finalized_block().await.unwrap_err();
        let deposit = api
            .fetch_deposit_events_at(Default::default(), 1, Default::default())
            .await
            .unwrap_err();
        server.abort();
        witness.abort();
        for error in [block, deposit] {
            assert_eq!(classify_anyhow(&error), RetryDecision::Retry, "{error:#}");
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
    fn does_not_retry_ambiguous_transaction_send_failures() {
        let err = ethereum_client::Error::ErrorSendingTransaction(
            alloy::contract::Error::TransportError(RpcError::NullResp),
        );

        assert_eq!(classify_ethereum_error(&err), RetryDecision::Fail);
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
    fn does_not_classify_untyped_backend_text_as_rpc() {
        let err = anyhow::anyhow!("backend connection task has stopped");

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
    }

    #[test]
    fn does_not_classify_untyped_socket_text_as_rpc() {
        let err = anyhow::anyhow!("transport error: connection refused");

        assert_eq!(classify_anyhow(&err), RetryDecision::Fail);
    }

    #[test]
    fn retries_unreachable_network_transports_but_not_rpc_rejections() {
        for kind in [
            std::io::ErrorKind::AddrNotAvailable,
            std::io::ErrorKind::NetworkDown,
            std::io::ErrorKind::NetworkUnreachable,
            std::io::ErrorKind::HostUnreachable,
        ] {
            let transport = RpcError::Transport(TransportErrorKind::Custom(Box::new(
                std::io::Error::from(kind),
            )));
            assert_eq!(
                classify_alloy_rpc(&transport),
                RetryDecision::Retry,
                "{kind:?}"
            );
            let rejection = RpcError::ErrorResp(
                serde_json::from_value(
                    serde_json::json!({"code": -32603, "message": format!("{kind:?}")}),
                )
                .unwrap(),
            );
            assert_eq!(classify_alloy_rpc(&rejection), RetryDecision::Fail);
            let send = ethereum_client::Error::ErrorSendingTransaction(
                alloy::contract::Error::TransportError(transport),
            );
            assert_eq!(classify_ethereum_error(&send), RetryDecision::Fail);
        }
    }

    #[tokio::test]
    async fn retry_stops_on_permanent_reconnect_failure_without_replaying_work() {
        let mut api = 17;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = retry(
            &mut api,
            "listener reconnect",
            RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
            None,
            |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Err::<(), _>("disconnected") }
            },
            |_| async { Err("unauthorized") },
            |error| {
                if *error == "disconnected" {
                    RetryDecision::Retry
                } else {
                    RetryDecision::Fail
                }
            },
        )
        .await;
        assert_eq!(result, Err("unauthorized"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(api, 17);
    }

    #[tokio::test]
    async fn retry_survives_a_recoverable_reconnect_failure() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let operation_calls = Arc::new(AtomicUsize::new(0));
        let reconnect_calls = Arc::new(AtomicUsize::new(0));
        let mut api = 0u32;
        let result = retry(
            &mut api,
            "test operation",
            RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
            Some(3),
            {
                let operation_calls = operation_calls.clone();
                move |api| {
                    let call = operation_calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if call == 0 {
                            Err("retry")
                        } else {
                            Ok(api + 10)
                        }
                    }
                }
            },
            {
                let reconnect_calls = reconnect_calls.clone();
                move |api| {
                    let call = reconnect_calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if call == 0 {
                            Err("retry")
                        } else {
                            Ok(api + 1)
                        }
                    }
                }
            },
            |_| RetryDecision::Retry,
        )
        .await
        .unwrap();

        assert_eq!(result, 11);
        assert_eq!(operation_calls.load(Ordering::SeqCst), 2);
        assert_eq!(reconnect_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bounded_retry_returns_after_its_reconnect_budget() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let reconnect_calls = Arc::new(AtomicUsize::new(0));
        let mut api = ();
        let err = retry(
            &mut api,
            "bounded test operation",
            RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
            Some(1),
            |_| async { Err::<(), _>("operation unavailable") },
            {
                let reconnect_calls = reconnect_calls.clone();
                move |_| {
                    reconnect_calls.fetch_add(1, Ordering::SeqCst);
                    async { Err("reconnect unavailable") }
                }
            },
            |_| RetryDecision::Retry,
        )
        .await
        .unwrap_err();

        assert_eq!(err, "reconnect unavailable");
        assert_eq!(reconnect_calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn bounded_retry_budget_is_fresh_for_each_operation() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        for _ in 0..2 {
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let mut api = 0u32;
            let result = retry(
                &mut api,
                "independent operation",
                RetryPolicy {
                    base_delay: Duration::ZERO,
                    max_delay: Duration::ZERO,
                },
                Some(1),
                move |_| {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if attempt == 0 {
                            Err("disconnected")
                        } else {
                            Ok(17)
                        }
                    }
                },
                |api| async move { Ok(api + 1) },
                |_| RetryDecision::Retry,
            )
            .await
            .unwrap();
            assert_eq!(result, 17);
            assert_eq!(observed.load(Ordering::SeqCst), 2);
        }
    }
}
