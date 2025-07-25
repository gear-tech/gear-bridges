use gclient::{GearApi, Error as GClientError, WSAddress};
use std::{env, process::Command, thread, time::Duration};

const NAME_CONTAINER: &str = "0197d24f-411b-73d2-ac12-727f50c401b2";

#[tokio::test]
async fn test_ws() {
    let port = env::var("TEST_TRANSPORT_ERRORS_PORT_WS")
        .ok()
        .and_then(|s| str::parse(&s).ok())
        .unwrap_or(65_100u16);

    test_inner(port, "ws").await;
}

#[tokio::test]
async fn test_http() {
    let port = env::var("TEST_TRANSPORT_ERRORS_PORT_HTTP")
        .ok()
        .and_then(|s| str::parse(&s).ok())
        .unwrap_or(65_101u16);

    test_inner(port, "http").await;
}

async fn test_inner(port: u16, protocol: &str) {
    let url = format!("{protocol}://127.0.0.1");
    let name = format!("{NAME_CONTAINER}-{port}");

    // 1. connection reset error
    docker_run(port, &name);
    let api = GearApi::init(WSAddress::new(&url, port)).await.unwrap();

    docker_stop(&name);

    let result = api.total_balance(api.account_id()).await;
    println!(r#"({protocol}) connection reset error: "{result:?}""#);
    let GClientError::GearSDK(gsdk::Error::Subxt(subxt::Error::Rpc(subxt::error::RpcError::ClientError(e)))) = result.err().unwrap() else {
        panic!("Not a ClientError, expected reset error");
    };
    let error_text = format!("{e:?}");
    assert!(error_text.starts_with("RestartNeeded(") || error_text.starts_with("Transport("), "{error_text}");

    // 2. timeout error
    docker_run(port, &name);
    let api = GearApi::init(WSAddress::new(&url, port)).await.unwrap();

    docker_pause(&name);

    let result = api.total_balance(api.account_id()).await;
    docker_stop(&name);
    println!(r#"({protocol}) timeout error: "{result:?}""#);
    let GClientError::GearSDK(gsdk::Error::Subxt(subxt::Error::Rpc(subxt::error::RpcError::ClientError(e)))) = result.err().unwrap() else {
        panic!("Not a ClientError, expected timeout error");
    };
    let error_text = format!("{e:?}");
    assert_eq!(error_text, "RequestTimeout");
}

fn docker_run(port: u16, name: &str) {
    let arg_publish = format!("127.0.0.1:{port}:9944");
    Command::new("docker")
        .args([
            "run",
            "--name",
            name,
            "--detach",
            "--rm",
            "--publish",
            &arg_publish,
            "ghcr.io/gear-tech/node:v1.8.1",
            "gear",
            "--dev",
            "--tmp",
            "--rpc-external",
        ])
        .output()
        .expect("failed to run docker container");

    thread::sleep(Duration::from_secs(10));
}

fn docker_pause(name: &str) {
    Command::new("docker")
        .args([
            "pause",
            name,
        ])
        .output()
        .expect("failed to pause docker container");
}

fn docker_stop(name: &str) {
    Command::new("docker")
        .args([
            "stop",
            name,
        ])
        .output()
        .expect("failed to stop docker container");
}
