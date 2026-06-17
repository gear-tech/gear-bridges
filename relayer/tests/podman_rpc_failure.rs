use std::{
    env,
    net::TcpListener,
    process::{Command, Output},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use ethereum_client::PollingEthApi;
use gear_common::api_provider::ApiProvider;
use relayer::rpc::{self, RetryDecision};

const DEFAULT_GEAR_ENDPOINT: &str = "wss://testnet-archive.vara.network";
const DEFAULT_GEAR_RPC_RETRIES: u8 = 3;
const DEFAULT_ETH_ENDPOINT: &str = "wss://hoodi-reth-rpc.gear-tech.io/ws";
const DEFAULT_WEBSOCAT_IMAGE: &str = "ghcr.io/vi/websocat:latest";

fn gear_endpoint() -> String {
    env::var("GEAR_ENDPOINT")
        .or_else(|_| env::var("GEAR_DOMAIN"))
        .unwrap_or_else(|_| DEFAULT_GEAR_ENDPOINT.to_string())
}

fn gear_retries() -> u8 {
    env::var("GEAR_RPC_RETRIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_GEAR_RPC_RETRIES)
}

fn eth_endpoint() -> String {
    env::var("ETHEREUM_RPC")
        .or_else(|_| env::var("ETHEREUM_ENDPOINT"))
        .or_else(|_| env::var("ETH_RPC"))
        .unwrap_or_else(|_| DEFAULT_ETH_ENDPOINT.to_string())
}

async fn gear_latest_number(
    connection: &mut gear_common::api_provider::ApiProviderConnection,
) -> anyhow::Result<u32> {
    rpc::retry_gear(
        connection,
        "podman test gear latest finalized",
        |api| async move {
            let hash = api.latest_finalized_block().await?;
            api.block_hash_to_number(hash).await
        },
    )
    .await
}

async fn eth_finalized_with_reconnect(api: &mut PollingEthApi) -> anyhow::Result<u64> {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), api.finalized_block()).await {
            Ok(Ok(block)) => return Ok(block.header.number),
            Ok(Err(err)) if rpc::classify_anyhow(&err) == RetryDecision::Retry => {
                reconnect_polling_api(api).await?;
            }
            Err(_) => reconnect_polling_api(api).await?,
            Ok(Err(err)) => return Err(err),
        }
    }
}

async fn reconnect_polling_api(api: &mut PollingEthApi) -> anyhow::Result<()> {
    tokio::time::sleep(Duration::from_millis(250)).await;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), api.reconnect()).await {
            Ok(Ok(reconnected)) => {
                *api = reconnected;
                return Ok(());
            }
            Ok(Err(reconnect_err))
                if rpc::classify_anyhow(&reconnect_err) == RetryDecision::Retry =>
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            Ok(Err(reconnect_err)) => return Err(reconnect_err),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires podman and live Vara testnet RPC"]
async fn podman_gear_worker_survives_real_proxy_outage_while_sibling_reconnects(
) -> anyhow::Result<()> {
    init_logging();
    let proxy = PodmanWebsocketProxy::start("gear", gear_endpoint()).await?;
    let provider = ApiProvider::new(proxy.local_url(), gear_retries()).await?;
    let mut setup_connection = provider.connection();
    let mut worker_connection = provider.connection();
    let mut reconnect_connection = provider.connection();
    provider.spawn();

    let baseline = gear_latest_number(&mut setup_connection).await?;
    proxy.stop_container().await?;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let proxy_for_restart = proxy.clone();
    let restarter = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(750)).await;
        proxy_for_restart.start_container().await
    });

    let worker = tokio::spawn(async move {
        let mut observed = Vec::new();
        for _ in 0..4 {
            eprintln!("gear worker requesting latest finalized block");
            observed.push(gear_latest_number(&mut worker_connection).await?);
            eprintln!("gear worker observed finalized block");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::Ok(observed)
    });

    let sibling_reconnector = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        eprintln!("gear sibling requesting provider reconnect");
        reconnect_connection.reconnect().await?;
        eprintln!("gear sibling provider reconnect returned");
        gear_latest_number(&mut reconnect_connection).await
    });

    tokio::time::timeout(Duration::from_secs(30), restarter)
        .await
        .context("timed out waiting for Gear proxy restart")???;
    let sibling_block = tokio::time::timeout(Duration::from_secs(90), sibling_reconnector)
        .await
        .context("timed out waiting for sibling Gear reconnect")???;
    let observed = tokio::time::timeout(Duration::from_secs(90), worker)
        .await
        .context("timed out waiting for Gear worker to resume")???;

    assert!(
        sibling_block >= baseline,
        "sibling reconnect saw finalized block regression: baseline={baseline}, sibling={sibling_block}"
    );
    assert_eq!(
        observed.len(),
        4,
        "worker did not finish all calls after real proxy outage"
    );
    assert!(
        observed.iter().all(|block| *block >= baseline),
        "worker saw finalized block regression after proxy outage: baseline={baseline}, observed={observed:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires podman and live Hoodi Ethereum RPC"]
async fn podman_ethereum_polling_recovers_after_real_proxy_outage() -> anyhow::Result<()> {
    init_logging();
    let proxy = PodmanWebsocketProxy::start("eth", eth_endpoint()).await?;
    let api = PollingEthApi::new_with_retries(
        &proxy.local_url(),
        Some(1),
        Some(Duration::from_millis(250)),
    )
    .await?;
    let mut worker_api = api.clone();
    let mut sibling_api = api;

    let baseline = worker_api.finalized_block().await?.header.number;
    proxy.stop_container().await?;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let proxy_for_restart = proxy.clone();
    let restarter = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(750)).await;
        proxy_for_restart.start_container().await
    });

    let worker = tokio::spawn(async move {
        let mut observed = Vec::new();
        for _ in 0..3 {
            observed.push(eth_finalized_with_reconnect(&mut worker_api).await?);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::Ok(observed)
    });

    let sibling_reconnector = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        eth_finalized_with_reconnect(&mut sibling_api).await
    });

    tokio::time::timeout(Duration::from_secs(30), restarter)
        .await
        .context("timed out waiting for Ethereum proxy restart")???;
    let sibling_block = tokio::time::timeout(Duration::from_secs(90), sibling_reconnector)
        .await
        .context("timed out waiting for sibling Ethereum reconnect")???;
    let observed = tokio::time::timeout(Duration::from_secs(90), worker)
        .await
        .context("timed out waiting for Ethereum worker to resume")???;

    assert!(
        sibling_block >= baseline,
        "sibling Ethereum reconnect saw finalized block regression: baseline={baseline}, sibling={sibling_block}"
    );
    assert!(
        observed.iter().all(|block| *block >= baseline),
        "Ethereum worker saw finalized block regression after proxy outage: baseline={baseline}, observed={observed:?}"
    );

    Ok(())
}

#[derive(Clone)]
struct PodmanWebsocketProxy {
    inner: Arc<PodmanWebsocketProxyInner>,
}

struct PodmanWebsocketProxyInner {
    name: String,
    port: u16,
    upstream: String,
    image: String,
}

impl PodmanWebsocketProxy {
    async fn start(label: &str, upstream: String) -> anyhow::Result<Self> {
        let image = env::var("PODMAN_WEBSOCAT_IMAGE")
            .unwrap_or_else(|_| DEFAULT_WEBSOCAT_IMAGE.to_string());
        ensure_image(&image).await?;

        let proxy = Self {
            inner: Arc::new(PodmanWebsocketProxyInner {
                name: unique_container_name(label),
                port: reserve_local_port()?,
                upstream,
                image,
            }),
        };
        proxy.start_container().await?;
        Ok(proxy)
    }

    fn local_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.inner.port)
    }

    async fn start_container(&self) -> anyhow::Result<()> {
        eprintln!(
            "starting podman proxy {} on ws://127.0.0.1:{} -> {}",
            self.inner.name, self.inner.port, self.inner.upstream
        );
        let _ = podman_output(vec!["rm".into(), "-f".into(), self.inner.name.clone()]).await;
        run_podman(vec![
            "run".into(),
            "--rm".into(),
            "-d".into(),
            "--name".into(),
            self.inner.name.clone(),
            "-p".into(),
            format!("127.0.0.1:{}:8080", self.inner.port),
            "--entrypoint".into(),
            "sh".into(),
            self.inner.image.clone(),
            "-c".into(),
            format!(
                "while true; do /usr/local/bin/websocat --text -B 4194304 ws-l:0.0.0.0:8080 {}; sleep 0.2; done",
                shell_quote(&self.inner.upstream)
            ),
        ])
        .await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(())
    }

    async fn stop_container(&self) -> anyhow::Result<()> {
        eprintln!("stopping podman proxy {}", self.inner.name);
        run_podman(vec![
            "stop".into(),
            "-t".into(),
            "0".into(),
            self.inner.name.clone(),
        ])
        .await
    }
}

impl Drop for PodmanWebsocketProxyInner {
    fn drop(&mut self) {
        let _ = Command::new("podman")
            .args(["rm", "-f", self.name.as_str()])
            .output();
    }
}

async fn ensure_image(image: &str) -> anyhow::Result<()> {
    if podman_output(vec!["image".into(), "exists".into(), image.into()])
        .await?
        .status
        .success()
    {
        return Ok(());
    }

    run_podman(vec!["pull".into(), image.into()]).await
}

async fn run_podman(args: Vec<String>) -> anyhow::Result<()> {
    let command = format!("podman {}", args.join(" "));
    let output = podman_output(args).await?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{command} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

async fn podman_output(args: Vec<String>) -> anyhow::Result<Output> {
    tokio::task::spawn_blocking(move || Command::new("podman").args(args).output())
        .await?
        .map_err(Into::into)
}

fn reserve_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn unique_container_name(label: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "gear-bridges-rpc-proxy-{label}-{}-{now}",
        std::process::id()
    )
}

fn init_logging() {
    let _ = pretty_env_logger::try_init_timed();
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
