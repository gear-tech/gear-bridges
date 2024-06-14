use axum::{routing::get, Router};
use prometheus::{core::Collector, Encoder, Registry, TextEncoder};
use tokio::net::TcpListener;

pub struct MetricsBuilder {
    registry: Registry,
}

pub struct Metrics {
    registry: Registry,
}

pub trait MeteredService {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn Collector>>;
}

impl MetricsBuilder {
    pub fn new() -> MetricsBuilder {
        MetricsBuilder {
            registry: Registry::new(),
        }
    }

    pub fn register_service(self, service: &impl MeteredService) -> Self {
        for source in service.get_sources() {
            self.registry
                .register(source)
                .expect("Failed to register metric source");
        }
        self
    }

    pub fn build(self) -> Metrics {
        Metrics {
            registry: self.registry,
        }
    }
}

impl Metrics {
    pub async fn run(self, endpoint: String) {
        let reg = self.registry.clone();

        let app = Router::new().route("/metrics", get(move || Self::gather_metrics(reg)));
        let listener = TcpListener::bind(&endpoint)
            .await
            .expect("Failed to create TcpListener");

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    async fn gather_metrics(registry: Registry) -> String {
        let mut buffer = vec![];

        let encoder = TextEncoder::new();
        let metric_families = registry.gather();
        encoder
            .encode(&metric_families, &mut buffer)
            .expect("Failed to encode metrics");

        String::from_utf8(buffer).expect("Failed to convert metrics to string")
    }
}
