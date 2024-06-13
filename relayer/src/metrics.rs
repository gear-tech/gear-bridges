use axum::{routing::get, Router};
use prometheus::{Encoder, Registry, TextEncoder};
use tokio::net::TcpListener;

lazy_static::lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
}

pub async fn run(endpoint: String) {
    // TODO: REGISTRY.register

    let app = Router::new().route("/metrics", get(gather_metrics));
    let listener = TcpListener::bind(&endpoint)
        .await
        .expect("Failed to create TcpListener");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
}

async fn gather_metrics() -> String {
    let mut buffer = vec![];

    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("Failed to encode metrics");

    String::from_utf8(buffer).expect("Failed to convert metrics to string")
}
